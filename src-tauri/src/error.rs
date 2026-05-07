use serde::Serialize;

pub(crate) type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl AppError {
    pub(crate) fn emit(error: tauri::Error) -> Self {
        Self {
            code: "event_emit_failed",
            message: format!("Failed to emit runtime event: {error}"),
        }
    }

    pub(crate) fn osc_encode(message: String) -> Self {
        Self {
            code: "osc_encode_failed",
            message: format!("Failed to encode OSC Chatbox message: {message}"),
        }
    }

    pub(crate) fn osc_bind(message: String) -> Self {
        Self {
            code: "osc_bind_failed",
            message: format!("Failed to open local UDP socket for OSC: {message}"),
        }
    }

    pub(crate) fn osc_send(target: &str, message: String) -> Self {
        Self {
            code: "osc_send_failed",
            message: format!("Failed to send OSC Chatbox message to {target}: {message}"),
        }
    }

    pub(crate) fn osc_send_incomplete(target: &str, expected: usize, sent: usize) -> Self {
        Self {
            code: "osc_send_incomplete",
            message: format!(
                "Sent an incomplete OSC datagram to {target}: {sent} of {expected} bytes"
            ),
        }
    }
}
