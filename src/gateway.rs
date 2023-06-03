use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use notify_rust::Notification;
#[cfg(target_os = "linux")]
use notify_rust::NotificationHandle;
use rand::{rngs::StdRng, Rng, SeedableRng};
use todel::models::{ClientPayload, Message, ServerPayload};
use tokio::sync::Mutex as AsyncMutex;
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
    token: String,
) {
    let rng = Arc::new(AsyncMutex::new(StdRng::from_entropy()));
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
            if let Some(Ok(WsMessage::Text(msg))) = rx.next().await {
                if let Ok(msg) = serde_json::from_str(&msg) {
                    match msg {
                        ServerPayload::Hello {
                            heartbeat_interval, ..
                        } => {
                            // Authenticate
                            if let Err(err) = tx
                                .send(WsMessage::Text(
                                    serde_json::to_string(&ClientPayload::Authenticate(
                                        token.clone(),
                                    ))
                                    .unwrap(),
                                ))
                                .await
                            {
                                messages.lock().unwrap().push((
                                    PilferMessage::System(SystemMessage {
                                        content: format!("Could not authenticate: {:?}", err),
                                    }),
                                    Style::default().fg(Color::Red),
                                ));
                                return;
                            }

                            // Handle ping-pong loop
                            let rng = Arc::clone(&rng);
                            ping = tokio::spawn(async move {
                                let dur = Duration::from_millis(
                                    rng.lock().await.gen_range(0..heartbeat_interval),
                                );
                                time::sleep(dur).await;
                                while let Ok(()) = tx
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&ClientPayload::Ping).unwrap(),
                                    ))
                                    .await
                                {
                                    time::sleep(Duration::from_millis(heartbeat_interval)).await;
                                }
                            });
                            break;
                        }
                        ServerPayload::RateLimit { wait } => {
                            messages.lock().unwrap().push((
                                PilferMessage::System(SystemMessage {
                                    content: format!("Rate limited, waiting {}s", wait / 1000),
                                }),
                                Style::default().fg(Color::Red),
                            ));
                            time::sleep(Duration::from_millis(wait)).await;
                        }
                        _ => continue,
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
                        Ok(ServerPayload::Authenticated) => {
                            messages.lock().unwrap().push((
                                PilferMessage::System(SystemMessage {
                                    content: "Authenticated with Pandemonium!".to_string(),
                                }),
                                Style::default().fg(Color::Green),
                            ));
                            continue;
                        }
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
