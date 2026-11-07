# WebSockets

Enable the `websocket` feature. Kerniq uses Axum's WebSocket implementation
(`kerniq_server::extract::ws`), which handles the upgrade, ping/pong and
framing.

```toml
kerniq-server = { version = "0.1", features = ["websocket"] }
```

## Echo server with graceful shutdown

```rust,no_run
use kerniq_server::extract::ws::{close_code, CloseFrame, Message, WebSocket, WebSocketUpgrade};
use kerniq_server::prelude::*;

#[derive(Clone)]
struct AppState {
    shutdown: Shutdown,
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| session(socket, state.shutdown))
}

async fn session(mut socket: WebSocket, shutdown: Shutdown) {
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    if socket.send(Message::Text(format!("echo: {text}").into())).await.is_err() {
                        break; // client went away
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {} // binary frames ignored; ping/pong is automatic
            },
            () = shutdown.wait() => {
                let frame = CloseFrame { code: close_code::AWAY, reason: "server shutting down".into() };
                let _ = socket.send(Message::Close(Some(frame))).await;
                break;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::new();
    let shutdown = app.shutdown_handle();
    app.state(AppState { shutdown })
        .get("/ws", ws_handler)
        .run("127.0.0.1:3000")
        .await
}
```

## Why the shutdown handle matters

After the upgrade, a WebSocket is no longer an HTTP request: Hyper's graceful
shutdown cannot see it. Without watching `Shutdown`, sockets would be cut when
the process exits. With it, every client receives a `1001 Going Away` close
frame and can reconnect elsewhere. The test
`crates/kerniq-server/tests/websocket.rs` verifies this.

## Patterns

* **Broadcast**: keep a `tokio::sync::broadcast::Sender` in state; each
  session subscribes and forwards messages.
* **Backpressure**: `socket.send(..).await` waits for the client; use
  `tokio::time::timeout` around sends to drop slow consumers.
* **Authentication**: authenticate the upgrade request with the `Auth`
  middleware or extractors before calling `on_upgrade`.
* **Limits**: `WebSocketUpgrade::max_message_size` and `max_frame_size`
  bound memory per connection.
