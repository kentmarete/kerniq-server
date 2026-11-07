//! WebSockets with graceful shutdown: on Ctrl+C every client receives a
//! `1001 Going Away` close frame before the server exits.
//!
//! ```sh
//! cargo run -p example-websocket
//! websocat ws://127.0.0.1:3000/ws
//! ```

use kerniq_server::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
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
                    let reply = format!("echo: {text}");
                    if socket.send(Message::Text(reply.into())).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {} // ping/pong handled by the library; binary ignored
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
    let app = app.state(AppState { shutdown }).get("/ws", ws_handler);

    println!("listening on ws://127.0.0.1:3000/ws");
    app.run("127.0.0.1:3000").await
}
