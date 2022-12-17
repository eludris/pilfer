use std::fmt::Display;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use notify_rust::NotificationHandle;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use todel::models::{ErrorResponse, Message};
use tui::style::Style;

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageResponse {
    Success(Message),
    Error(ErrorResponse),
}

// While in hindsight this might look like it's modeled in a bad way, you're right, it's modeled in
// a bad way.
#[derive(Debug)]
pub struct SystemMessage {
    pub content: String,
}

#[derive(Debug)]
pub enum PilferMessage {
    Eludris(Message),
    System(SystemMessage),
}

impl Display for PilferMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PilferMessage::Eludris(msg) => write!(f, "[{}]: {}", msg.author, msg.content),
            PilferMessage::System(msg) => write!(f, "{}", msg.content),
        }
    }
}

pub struct AppContext {
    /// Current input
    pub input: String,
    /// User name
    pub name: String,
    /// Received messages
    pub messages: Arc<Mutex<Vec<(PilferMessage, Style)>>>,
    /// Reqwest HttpClient
    pub http_client: Client,
    /// Oprish URL
    pub rest_url: String,
    /// Whether the user is currently focused.
    pub focused: Arc<AtomicBool>,
    /// The notification
    #[cfg(target_os = "linux")]
    pub notification: Arc<Mutex<Option<NotificationHandle>>>,
}
