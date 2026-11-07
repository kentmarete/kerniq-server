# Databases and message brokers

Kerniq has no built-in database layer, ORM or broker client. Use the Rust
crate you would use with Axum or Tokio (`sqlx`, `redis`, `mongodb`,
`rdkafka`, `lapin`, …) and connect it to the app the same way every time:

1. **Connect once, at startup**, before the server starts listening. A bad
   URL or an unreachable database is a startup error, not a failed request.
2. **Put the client in the [shared state](state-management.md).** Pools and
   clients are cheap `Arc` handles, so cloning per request is free.
3. **Map client errors to Kerniq `Error`s** in handlers: expected cases
   (not found, duplicate key) become 4xx responses, everything else
   `.or_internal()`, which logs the cause and never leaks it to the client.
4. **Run consumers as [background tasks](runtime.md#background-tasks)**
   that stop when the `Shutdown` handle fires.
5. **Close connections after `run` returns.** `run` only returns once
   in-flight requests have drained and background tasks have finished.

The skeleton, with a stand-in client:

```rust,no_run
use std::time::Duration;
use kerniq_server::prelude::*;

#[derive(Clone)]
struct Db; // e.g. sqlx::PgPool, redis::aio::ConnectionManager, mongodb::Client

impl Db {
    async fn connect(_url: &str) -> std::io::Result<Self> { Ok(Db) }
    async fn ping(&self) -> std::io::Result<()> { Ok(()) }
    async fn close(&self) {}
}

#[derive(Clone)]
struct AppState {
    db: Db,
}

async fn ready(State(state): State<AppState>) -> Result<&'static str> {
    state
        .db
        .ping()
        .await
        .or_error(Error::service_unavailable("Database unavailable"))?;
    Ok("READY")
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // 1. Connect at startup; fail fast on bad configuration.
    let url = std::env::var("DATABASE_URL")?;
    let db = Db::connect(&url).await?;

    // 2. Share it through state.
    let app = App::new()
        .state(AppState { db: db.clone() })
        .get("/health/ready", ready)
        // 4. Consumers and pollers run as background tasks.
        .background_task("consumer", |shutdown: Shutdown| async move {
            loop {
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(1)) => { /* poll / consume */ }
                    () = shutdown.wait() => break,
                }
            }
        });

    app.run("0.0.0.0:3000").await?;

    // 5. Requests have drained and tasks have stopped: release connections.
    db.close().await;
    Ok(())
}
```

`main` returns `Box<dyn std::error::Error>` so that `?` works on both the
client's startup errors and Kerniq's `Error`.

The sections below fill in the skeleton for common backends. They need
third-party crates, so they are marked `ignore` and are not compiled as part
of Kerniq's test suite; they were checked against the crate versions shown.

* [PostgreSQL, MySQL and SQLite (`sqlx`)](#postgresql-mysql-and-sqlite-sqlx)
* [Redis](#redis)
* [MongoDB](#mongodb)
* [Kafka](#kafka)
* [RabbitMQ](#rabbitmq)
* [Configuration and secrets](#configuration-and-secrets)
* [Testing](#testing)

## PostgreSQL, MySQL and SQLite (`sqlx`)

```toml
[dependencies]
sqlx = { version = "0.9", features = ["runtime-tokio", "tls-rustls", "postgres"] }
# MySQL: "mysql" instead of "postgres". SQLite: "sqlite".
```

```rust,ignore
use std::time::Duration;

use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Clone)]
struct AppState {
    db: PgPool,
}

#[derive(Serialize, sqlx::FromRow)]
struct User {
    id: i64,
    email: String,
}

#[derive(Deserialize)]
struct NewUser {
    email: String,
}

/// Turns expected database errors into client errors; everything else is a
/// logged, opaque 500.
fn db_error(err: sqlx::Error) -> Error {
    match &err {
        sqlx::Error::RowNotFound => Error::not_found("User not found"),
        sqlx::Error::Database(e) if e.is_unique_violation() => {
            Error::conflict("Email already registered").with_code("EMAIL_TAKEN")
        }
        _ => Error::internal(err),
    }
}

async fn get_user(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<User>> {
    let user = sqlx::query_as::<_, User>("SELECT id, email FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(db_error)?;
    Ok(Json(user))
}

async fn create_user(State(state): State<AppState>, Json(input): Json<NewUser>) -> Result<Created<User>> {
    let user = sqlx::query_as::<_, User>("INSERT INTO users (email) VALUES ($1) RETURNING id, email")
        .bind(&input.email)
        .fetch_one(&state.db)
        .await
        .map_err(db_error)?;
    Ok(Created(user))
}

async fn ready(State(state): State<AppState>) -> Result<&'static str> {
    sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .or_error(Error::service_unavailable("Database unavailable"))?;
    Ok("READY")
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let db = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;

    // Optional: apply migrations from ./migrations at startup
    // (requires sqlx's "migrate" feature).
    // sqlx::migrate!().run(&db).await?;

    App::new()
        .state(AppState { db: db.clone() })
        .get("/health/ready", ready)
        .get("/users/{id}", get_user)
        .post("/users", create_user)
        .run("0.0.0.0:3000")
        .await?;

    db.close().await;
    Ok(())
}
```

Notes:

* Size the pool together with `ServerConfig::max_concurrent_requests`. If
  1024 requests can run at once but the pool holds 20 connections, the rest
  wait in `acquire` until `acquire_timeout`, then fail. A request timeout
  shorter than the acquire timeout turns that wait into a `408`.
* Transactions: `let mut tx = state.db.begin().await.or_internal()?;`, run
  queries against `&mut *tx`, then `tx.commit().await.or_internal()?`. A
  transaction dropped without `commit` (including by an early `?` or a
  request timeout) is rolled back.
* `sqlx::query!` / `query_as!` check SQL at compile time against a live
  database or an offline `.sqlx` cache (`cargo sqlx prepare`).
* Diesel users can use `diesel-async` with a `deadpool` or `bb8` pool the
  same way: put the pool in state and call `pool.get().await` in handlers.

## Redis

```toml
[dependencies]
redis = { version = "1", features = ["tokio-comp", "connection-manager"] }
```

`ConnectionManager` is a multiplexed connection that reconnects
automatically, and it is cheap to clone, so it goes straight into state.

```rust,ignore
use kerniq_server::prelude::*;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;

#[derive(Clone)]
struct AppState {
    redis: ConnectionManager,
}

async fn get_value(State(state): State<AppState>, Path(key): Path<String>) -> Result<String> {
    let mut redis = state.redis.clone();
    let value: Option<String> = redis.get(&key).await.or_internal()?;
    value.or_not_found("Key not found")
}

async fn put_value(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: String,
) -> Result<NoContent> {
    let mut redis = state.redis.clone();
    // Expire after one hour.
    let () = redis.set_ex(&key, body, 3600).await.or_internal()?;
    Ok(NoContent)
}

async fn ready(State(state): State<AppState>) -> Result<&'static str> {
    let mut redis = state.redis.clone();
    let _: String = redis::cmd("PING")
        .query_async(&mut redis)
        .await
        .or_error(Error::service_unavailable("Redis unavailable"))?;
    Ok("READY")
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let client = redis::Client::open(std::env::var("REDIS_URL")?)?; // redis://127.0.0.1:6379
    let redis = ConnectionManager::new(client).await?;

    App::new()
        .state(AppState { redis })
        .get("/health/ready", ready)
        .get("/kv/{key}", get_value)
        .put("/kv/{key}", put_value)
        .run("0.0.0.0:3000")
        .await?;
    Ok(())
}
```

**Pub/sub** needs a dedicated connection. Run the subscriber as a background
task:

```rust,ignore
use futures_util::StreamExt;
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let client = redis::Client::open(std::env::var("REDIS_URL")?)?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe("events").await?;

    App::new()
        .background_task("redis-events", |shutdown: Shutdown| async move {
            let mut messages = pubsub.on_message();
            loop {
                tokio::select! {
                    Some(msg) = messages.next() => {
                        let payload: String = msg.get_payload().unwrap_or_default();
                        tracing::info!(channel = msg.get_channel_name(), %payload, "event");
                    }
                    () = shutdown.wait() => break,
                    else => break, // connection closed
                }
            }
        })
        .get("/", || async { "OK" })
        .run("0.0.0.0:3000")
        .await?;
    Ok(())
}
```

Subscribing before `run` means a bad URL fails startup instead of silently
killing the task.

## MongoDB

```toml
[dependencies]
mongodb = "3"
```

```rust,ignore
use kerniq_server::prelude::*;
use mongodb::bson::doc;
use mongodb::{Client, Collection};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct Product {
    sku: String,
    name: String,
}

#[derive(Clone)]
struct AppState {
    products: Collection<Product>,
}

async fn get_product(State(state): State<AppState>, Path(sku): Path<String>) -> Result<Json<Product>> {
    let product = state
        .products
        .find_one(doc! { "sku": &sku })
        .await
        .or_internal()?
        .or_not_found("Product not found")?;
    Ok(Json(product))
}

async fn create_product(State(state): State<AppState>, Json(product): Json<Product>) -> Result<Created<Product>> {
    state.products.insert_one(&product).await.or_internal()?;
    Ok(Created(product))
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // The client is a connection pool; connect lazily and check it once.
    let client = Client::with_uri_str(std::env::var("MONGODB_URL")?).await?;
    let db = client.database("shop");
    db.run_command(doc! { "ping": 1 }).await?;

    App::new()
        .state(AppState { products: db.collection("products") })
        .get("/products/{sku}", get_product)
        .post("/products", create_product)
        .run("0.0.0.0:3000")
        .await?;

    client.shutdown().await;
    Ok(())
}
```

## Kafka

```toml
[dependencies]
rdkafka = { version = "0.39", features = ["cmake-build"] }
```

`rdkafka` wraps the C library `librdkafka`. `cmake-build` compiles it from
source (needs `cmake` and a C compiler); alternatively, use the `dynamic-linking`
feature with a system `librdkafka`.

The example publishes from an HTTP handler and consumes in a background
task:

```rust,ignore
use std::time::Duration;

use kerniq_server::prelude::*;
use rdkafka::ClientConfig;
use rdkafka::Message;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::producer::{FutureProducer, FutureRecord, Producer};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct AppState {
    producer: FutureProducer,
}

#[derive(Serialize, Deserialize)]
struct Order {
    id: String,
    amount_cents: i64,
}

async fn publish_order(State(state): State<AppState>, Json(order): Json<Order>) -> Result<StatusCode> {
    let payload = serde_json::to_vec(&order)?;
    state
        .producer
        .send(
            FutureRecord::to("orders").key(&order.id).payload(&payload),
            Duration::from_secs(5), // how long to wait if the local queue is full
        )
        .await
        .map_err(|(err, _message)| {
            Error::service_unavailable("Could not publish order").with_source(err)
        })?;
    Ok(StatusCode::ACCEPTED)
}

async fn consume_orders(consumer: StreamConsumer, shutdown: Shutdown) {
    loop {
        tokio::select! {
            received = consumer.recv() => match received {
                Ok(message) => {
                    if let Some(Ok(payload)) = message.payload_view::<str>() {
                        match serde_json::from_str::<Order>(payload) {
                            Ok(order) => tracing::info!(id = %order.id, "order received"),
                            Err(err) => tracing::warn!(%err, "skipping malformed order"),
                        }
                    }
                    // Commit only after the message was processed.
                    if let Err(err) = consumer.commit_message(&message, CommitMode::Async) {
                        tracing::warn!(%err, "commit failed");
                    }
                }
                Err(err) => tracing::error!(%err, "kafka error"),
            },
            () = shutdown.wait() => break,
        }
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let brokers = std::env::var("KAFKA_BROKERS")?; // e.g. "localhost:9092"

    let producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .set("message.timeout.ms", "5000")
        .set("enable.idempotence", "true")
        .create()?;

    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .set("group.id", "order-service")
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .create()?;
    consumer.subscribe(&["orders"])?;

    App::new()
        .state(AppState { producer: producer.clone() })
        .post("/orders", publish_order)
        .background_task("kafka-orders", |shutdown: Shutdown| consume_orders(consumer, shutdown))
        .run("0.0.0.0:3000")
        .await?;

    // Deliver anything still queued before exiting.
    producer.flush(Duration::from_secs(10))?;
    Ok(())
}
```

Notes:

* `FutureProducer` is a cheap `Clone` handle; share one per process.
* Kafka delivers at least once. Make consumers idempotent (for example, by
  storing processed message keys or offsets with the result).
* Background tasks get `ServerConfig::shutdown_timeout` to finish, so keep
  per-message work shorter than that, or check `shutdown.is_triggered()`
  inside long loops.

## RabbitMQ

```toml
[dependencies]
lapin = "4"
futures-util = "0.3"
```

`lapin` is an AMQP 0.9.1 client. Open one connection per process and one
channel per concurrent user of it: a channel for publishing from handlers,
another for the consumer.

```rust,ignore
use futures_util::StreamExt;
use kerniq_server::prelude::*;
use lapin::options::{
    BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicPublishOptions,
    QueueDeclareOptions,
};
use lapin::types::FieldTable;
use lapin::{BasicProperties, Channel, Connection, ConnectionProperties, Consumer};

const QUEUE: &str = "emails";

#[derive(Clone)]
struct AppState {
    channel: Channel,
}

async fn enqueue_email(State(state): State<AppState>, body: String) -> Result<StatusCode> {
    state
        .channel
        .basic_publish(
            "".into(),    // default exchange
            QUEUE.into(), // routing key = queue name
            BasicPublishOptions::default(),
            body.as_bytes(),
            BasicProperties::default().with_delivery_mode(2), // persistent
        )
        .await
        .or_internal()?
        .await // wait for the broker confirm (when confirms are enabled)
        .or_internal()?;
    Ok(StatusCode::ACCEPTED)
}

async fn consume_emails(mut consumer: Consumer, shutdown: Shutdown) {
    loop {
        tokio::select! {
            delivery = consumer.next() => match delivery {
                Some(Ok(delivery)) => {
                    let body = String::from_utf8_lossy(&delivery.data);
                    tracing::info!(%body, "sending email");
                    let ok = true; // ... do the work ...
                    let result = if ok {
                        delivery.ack(BasicAckOptions::default()).await
                    } else {
                        delivery.nack(BasicNackOptions { requeue: true, ..Default::default() }).await
                    };
                    if let Err(err) = result {
                        tracing::error!(%err, "ack failed");
                    }
                }
                Some(Err(err)) => tracing::error!(%err, "rabbitmq error"),
                None => break, // channel closed
            },
            () = shutdown.wait() => break,
        }
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("AMQP_URL")?; // amqp://guest:guest@localhost:5672/%2f
    let connection = Connection::connect(&url, ConnectionProperties::default()).await?;

    let publish_channel = connection.create_channel().await?;
    publish_channel
        .queue_declare(QUEUE.into(), QueueDeclareOptions { durable: true, ..Default::default() }, FieldTable::default())
        .await?;

    let consume_channel = connection.create_channel().await?;
    consume_channel.basic_qos(10, Default::default()).await?; // at most 10 unacked messages
    let consumer = consume_channel
        .basic_consume(QUEUE.into(), "email-worker".into(), BasicConsumeOptions::default(), FieldTable::default())
        .await?;

    App::new()
        .state(AppState { channel: publish_channel })
        .post("/emails", enqueue_email)
        .background_task("rabbitmq-emails", |shutdown: Shutdown| consume_emails(consumer, shutdown))
        .run("0.0.0.0:3000")
        .await?;

    connection.close(200, "shutting down".into()).await?;
    Ok(())
}
```

Notes:

* Messages that are neither acked nor nacked are redelivered when the
  channel closes, so stopping mid-message on shutdown does not lose work.
* Enable publisher confirms with
  `channel.confirm_select(Default::default()).await?` to know the broker has
  stored a message before responding `202`.
* `lapin` does not reconnect automatically. For long-running services,
  watch `connection.status()` in a readiness probe so the orchestrator
  restarts the instance, or wrap connection setup in a reconnect loop.

## Configuration and secrets

`ServerConfig` covers server settings only (see
[Configuration](configuration.md)). Keep connection URLs in environment
variables or your own config struct, and never log them: they usually
contain credentials. In the production template (`kerniq new --template
production`), add them to `src/config.rs` and `.env.example`.

## Testing

* Handlers depend on state, not on globals, so tests can build the app with
  a test database and call it in-process through `into_axum_router()` (see
  [Axum and Tower interoperability](axum-interop.md)).
* `sqlx` provides `#[sqlx::test]`, which gives each test a fresh database.
* For brokers and Redis, run real instances in CI (Docker services or
  `testcontainers`) rather than mocking the protocol.
