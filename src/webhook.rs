use crate::output;
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    SessionStart,
    SessionComplete,
    SessionFailed,
}

impl EventType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::SessionComplete => "session_complete",
            Self::SessionFailed => "session_failed",
        }
    }
}

#[derive(Serialize)]
struct WebhookPayload<'a> {
    event: &'a str,
    timestamp: String,
    message: &'a str,
}

pub fn send_webhook(url: &str, event: EventType, message: &str) {
    let url = url.to_string();
    let event_str = event.as_str();
    let message = message.to_string();

    tokio::spawn(async move {
        let payload = WebhookPayload {
            event: event_str,
            timestamp: Utc::now().to_rfc3339(),
            message: &message,
        };

        let client = reqwest::Client::new();
        match client.post(&url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                output::dim(&format!("Webhook sent: {event_str}"));
            }
            Ok(resp) => {
                output::warn(&format!(
                    "Webhook returned {}: {event_str}",
                    resp.status()
                ));
            }
            Err(e) => {
                output::warn(&format!("Webhook failed: {e}"));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_as_str() {
        assert_eq!(EventType::SessionStart.as_str(), "session_start");
        assert_eq!(EventType::SessionComplete.as_str(), "session_complete");
        assert_eq!(EventType::SessionFailed.as_str(), "session_failed");
    }

    #[test]
    fn event_type_equality() {
        assert_eq!(EventType::SessionStart, EventType::SessionStart);
        assert_ne!(EventType::SessionStart, EventType::SessionComplete);
    }

    #[test]
    fn webhook_payload_serialization() {
        let payload = WebhookPayload {
            event: "session_start",
            timestamp: "2024-01-15T10:30:00Z".to_string(),
            message: "Starting session",
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"event\":\"session_start\""));
        assert!(json.contains("\"timestamp\":\"2024-01-15T10:30:00Z\""));
        assert!(json.contains("\"message\":\"Starting session\""));
    }
}
