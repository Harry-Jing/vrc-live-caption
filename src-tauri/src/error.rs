use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};
use std::error::Error;
use std::fmt;

pub(crate) type AppResult<T> = Result<T, AppError>;

#[derive(Debug)]
pub(crate) enum AppError {
    Audio {
        message: String,
    },
    Config {
        message: String,
    },
    ConfigIo {
        message: String,
    },
    EventEmit {
        source: tauri::Error,
    },
    OscEncode {
        message: String,
    },
    OscBind {
        message: String,
    },
    OscSend {
        target: String,
        message: String,
    },
    OscSendIncomplete {
        target: String,
        expected: usize,
        sent: usize,
    },
    Runtime {
        message: String,
    },
    Secret {
        message: String,
    },
    State {
        message: String,
    },
    Stt {
        message: String,
    },
    Wav {
        message: String,
    },
}

impl AppError {
    pub(crate) fn audio(message: impl Into<String>) -> Self {
        Self::Audio {
            message: message.into(),
        }
    }

    pub(crate) fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
        }
    }

    pub(crate) fn config_io(message: impl Into<String>) -> Self {
        Self::ConfigIo {
            message: message.into(),
        }
    }

    pub(crate) fn emit(error: tauri::Error) -> Self {
        Self::EventEmit { source: error }
    }

    pub(crate) fn osc_encode(message: String) -> Self {
        Self::OscEncode { message }
    }

    pub(crate) fn osc_bind(message: String) -> Self {
        Self::OscBind { message }
    }

    pub(crate) fn osc_send(target: &str, message: String) -> Self {
        Self::OscSend {
            target: target.to_string(),
            message,
        }
    }

    pub(crate) fn osc_send_incomplete(target: &str, expected: usize, sent: usize) -> Self {
        Self::OscSendIncomplete {
            target: target.to_string(),
            expected,
            sent,
        }
    }

    pub(crate) fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime {
            message: message.into(),
        }
    }

    pub(crate) fn secret(message: impl Into<String>) -> Self {
        Self::Secret {
            message: message.into(),
        }
    }

    pub(crate) fn state(message: impl Into<String>) -> Self {
        Self::State {
            message: message.into(),
        }
    }

    pub(crate) fn stt(message: impl Into<String>) -> Self {
        Self::Stt {
            message: message.into(),
        }
    }

    pub(crate) fn wav(message: impl Into<String>) -> Self {
        Self::Wav {
            message: format!("Failed to encode captured audio as WAV: {}", message.into()),
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Audio { .. } => "audio_failed",
            Self::Config { .. } => "config_invalid",
            Self::ConfigIo { .. } => "config_io_failed",
            Self::EventEmit { .. } => "event_emit_failed",
            Self::OscEncode { .. } => "osc_encode_failed",
            Self::OscBind { .. } => "osc_bind_failed",
            Self::OscSend { .. } => "osc_send_failed",
            Self::OscSendIncomplete { .. } => "osc_send_incomplete",
            Self::Runtime { .. } => "runtime_failed",
            Self::Secret { .. } => "secret_failed",
            Self::State { .. } => "state_failed",
            Self::Stt { .. } => "stt_failed",
            Self::Wav { .. } => "wav_encode_failed",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Audio { message } => message.clone(),
            Self::Config { message } => message.clone(),
            Self::ConfigIo { message } => message.clone(),
            Self::EventEmit { source } => {
                format!("Failed to emit runtime event: {source}")
            }
            Self::OscEncode { message } => {
                format!("Failed to encode OSC Chatbox message: {message}")
            }
            Self::OscBind { message } => {
                format!("Failed to open local UDP socket for OSC: {message}")
            }
            Self::OscSend { target, message } => {
                format!("Failed to send OSC Chatbox message to {target}: {message}")
            }
            Self::OscSendIncomplete {
                target,
                expected,
                sent,
            } => format!("Sent an incomplete OSC datagram to {target}: {sent} of {expected} bytes"),
            Self::Runtime { message } => message.clone(),
            Self::Secret { message } => message.clone(),
            Self::State { message } => message.clone(),
            Self::Stt { message } => message.clone(),
            Self::Wav { message } => message.clone(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EventEmit { source } => Some(source),
            _ => None,
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.message())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_error_serializes_with_stable_code_and_message() {
        let error = AppError::osc_send("127.0.0.1:9000", "network unreachable".to_string());
        let value = serde_json::to_value(&error).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

        assert_eq!(value["code"], "osc_send_failed");
        assert_eq!(
            value["message"],
            "Failed to send OSC Chatbox message to 127.0.0.1:9000: network unreachable"
        );
    }
}
