#![cfg(feature = "websocket")]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kerniq_server::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use kerniq_server::prelude::*;
use tokio_tungstenite::tungstenite;

#[derive(Clone)]
struct AppState {
    shutdown: Shutdown,
}

async fn echo(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| session(socket, state.shutdown))
}

/// Echoes text messages until the client leaves or the server shuts down,
/// in which case the client receives a proper close frame.
async fn session(mut socket: WebSocket, shutdown: Shutdown) {
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    if socket.send(Message::Text(format!("echo: {text}").into())).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            () = shutdown.wait() => {
                let _ = socket
                    .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::AWAY,
                        reason: "server shutting down".into(),
                    })))
                    .await;
                break;
            }
        }
    }
}

#[tokio::test]
async fn websocket_echo_and_graceful_close_on_shutdown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = App::new();
    let shutdown = app.shutdown_handle();
    let app = app
        .state(AppState {
            shutdown: shutdown.clone(),
        })
        .get("/ws", echo);
    let server = tokio::spawn(app.serve(listener));

    let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    client
        .send(tungstenite::Message::Text("hello".into()))
        .await
        .unwrap();
    let reply = client.next().await.unwrap().unwrap();
    assert_eq!(reply.into_text().unwrap().as_str(), "echo: hello");

    shutdown.trigger();
    let frame = tokio::time::timeout(Duration::from_secs(5), client.next())
        .await
        .expect("close frame arrives")
        .unwrap()
        .unwrap();
    match frame {
        tungstenite::Message::Close(Some(close)) => {
            assert_eq!(u16::from(close.code), 1001);
            assert_eq!(close.reason.as_str(), "server shutting down");
        }
        other => panic!("expected close frame, got {other:?}"),
    }
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
