use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use notify_rust::Notification;
#[cfg(target_os = "linux")]
use notify_rust::NotificationHandle;
use todel::models::{ClientPayload, Message, ServerPayload};
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tui::style::{Color, Style};

use crate::models::{PilferMessage, SystemMessage};

pub async fn handle_gateway(
    gateway_url: String,
    messages: Arc<Mutex<Vec<(PilferMessage, Style)>>>,
    focused: Arc<AtomicBool>,
    #[cfg(target_os = "linux")] notification: Arc<Mutex<Option<NotificationHandle>>>,
    name: String,
) {
    let mut wait = 0;
    loop {
        if wait > 0 {
            time::sleep(Duration::from_secs(wait)).await;
        }

        let socket = match connect_async(&gateway_url).await {
            Ok((socket, _)) => socket,
            Err(err) => {
                if wait < 64 {
                    wait *= 2;
                }
                messages.lock().unwrap().push((
                    PilferMessage::System(SystemMessage {
                        content: format!(
                            "Could not connect: {:?}, reconnecting in {}s (press Ctrl+C to exit)",
                            err, wait
                        ),
                    }),
                    Style::default().fg(Color::Red),
                ));
                continue;
            }
        };
        wait = 0;

        let (mut tx, mut rx) = socket.split();
        let ping;
        loop {
            if let Some(Ok(msg)) = rx.next().await {
                if let WsMessage::Text(msg) = msg {
                    if let Ok(ServerPayload::Hello {
                        heartbeat_interval, ..
                    }) = serde_json::from_str(&msg)
                    {
                        // Handle ping-pong loop
                        ping = tokio::spawn(async move {
                            while let Ok(()) = tx
                                .send(WsMessage::Text(
                                    serde_json::to_string(&ClientPayload::Ping).unwrap(),
                                ))
                                .await
                            {
                                time::sleep(Duration::from_secs(heartbeat_interval)).await;
                            }
                        });
                        break;
                    }
                }
            }
        }

        messages.lock().unwrap().push((
            PilferMessage::System(SystemMessage {
                content: "Connected to Pandemonium".to_string(),
            }),
            Style::default().fg(Color::Green),
        ));

        // Handle receiving pandemonium events
        while let Some(Ok(msg)) = rx.next().await {
            match msg {
                WsMessage::Text(msg) => {
                    let msg: Message = match serde_json::from_str(&msg) {
                        Ok(ServerPayload::MessageCreate(msg)) => msg,
                        _ => continue,
                    };
                    if !focused.load(std::sync::atomic::Ordering::Relaxed) {
                        #[cfg(target_os = "linux")]
                        {
                            let mut notif = notification.lock().unwrap();
                            match notif.as_mut() {
                                Some(notif) => {
                                    notif
                                        .summary(&format!("New Pilfer message from {}", msg.author))
                                        .body(&msg.content.to_string());
                                    notif.update()
                                }
                                None => {
                                    *notif = match Notification::new()
                                        .summary(&format!("New Pilfer message from {}", msg.author))
                                        .body(&msg.content)
                                        .show()
                                    {
                                        Ok(notif) => Some(notif),
                                        Err(_) => None,
                                    };
                                }
                            }
                        }
                        #[cfg(not(target_os = "linux"))]
                        Notification::new()
                            .summary(&format!("New Pilfer message from {}", msg.author))
                            .body(&msg.content)
                            .show()
                            .ok();
                    }
                    // Highlight the message if your name got mentioned
                    let style = if msg.content.to_lowercase().contains(&name.to_lowercase()) {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    // Add to the Pifler's context
                    messages
                        .lock()
                        .unwrap()
                        .push((PilferMessage::Eludris(msg), style));
                }
                WsMessage::Close(frame) => {
                    if let Some(frame) = frame {
                        if wait < 64 {
                            wait *= 2;
                        }
                        messages.lock().unwrap().push((
                            PilferMessage::System(SystemMessage {
                                content: format!("{}, retrying in {}s", frame.reason, wait),
                            }),
                            Style::default().fg(Color::Red),
                        ))
                    }
                    ping.abort();
                    continue;
                }
                _ => {}
            }
        }
    }
}
