//! Application error type shared by Tauri commands and the runtime.
//!
//! `AppError` serializes to `{ code, message }` for the frontend: `code` is a
//! stable machine-readable identifier following the diagnostic naming
//! convention documented in `events`, and `message` is human-readable text.
//! Variants flatten their causes into the message because the error crosses
//! the IPC boundary as JSON.

use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};
use std::error::Error;
use std::fmt;

pub(crate) type AppResult<T> = Result<T, AppError>;

/// Stable, provider-neutral classification for a provider-originated STT
/// failure. Provider wire strings are mapped to this closed set inside the
/// concrete adapter and never cross into `AppError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderFailureClass {
    Authentication,
    PermissionDenied,
    InvalidRequest,
    RateLimited,
    UsageLimit,
    ServiceUnavailable,
    Unknown,
}

/// Whether retrying the same operation may succeed without changing user
/// configuration. Runtime policy still owns delay, attempt, and total-time
/// limits for retryable failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryDisposition {
    Retryable,
    Terminal,
}

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
    SttProvider {
        class: ProviderFailureClass,
        /// Compile-time text authored by this application. Keeping this
        /// `&'static str` prevents provider strings from entering the error by
        /// accident.
        message: &'static str,
    },
    SttBackpressure {
        message: String,
    },
    SttNetwork {
        message: String,
        retry_disposition: RetryDisposition,
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

    pub(crate) fn stt_provider(class: ProviderFailureClass, message: &'static str) -> Self {
        Self::SttProvider { class, message }
    }

    pub(crate) fn stt_backpressure(message: impl Into<String>) -> Self {
        Self::SttBackpressure {
            message: message.into(),
        }
    }

    pub(crate) fn stt_network(message: impl Into<String>) -> Self {
        Self::SttNetwork {
            message: message.into(),
            retry_disposition: RetryDisposition::Terminal,
        }
    }

    pub(crate) fn stt_network_retryable(message: impl Into<String>) -> Self {
        Self::SttNetwork {
            message: message.into(),
            retry_disposition: RetryDisposition::Retryable,
        }
    }

    /// Machine-readable code for logs and diagnostic events. Codes follow the
    /// diagnostic naming convention `<category>.<detail>`, where the prefix
    /// matches the serialized category chosen by
    /// `DiagnosticCategory::for_error`.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Audio { .. } => "audio.failed",
            Self::Config { .. } => "config.invalid",
            Self::ConfigIo { .. } => "config.io_failed",
            Self::OscEncode { .. } => "osc.encode_failed",
            Self::OscBind { .. } => "osc.bind_failed",
            Self::OscSend { .. } => "osc.send_failed",
            Self::OscSendIncomplete { .. } => "osc.send_incomplete",
            Self::Runtime { .. } => "runtime.failed",
            Self::Secret { .. } => "config.secret_failed",
            Self::State { .. } => "runtime.state_failed",
            Self::Stt { .. } => "stt.failed",
            Self::SttProvider { class, .. } => match class {
                ProviderFailureClass::Authentication => "stt.provider_authentication_failed",
                ProviderFailureClass::PermissionDenied => "stt.provider_permission_denied",
                ProviderFailureClass::InvalidRequest => "stt.provider_invalid_request",
                ProviderFailureClass::RateLimited => "stt.provider_rate_limited",
                ProviderFailureClass::UsageLimit => "stt.provider_usage_limit",
                ProviderFailureClass::ServiceUnavailable => "stt.provider_unavailable",
                ProviderFailureClass::Unknown => "stt.provider_failed",
            },
            Self::SttBackpressure { .. } => "stt.backpressure",
            Self::SttNetwork { .. } => "stt.network_unreachable",
        }
    }

    pub(crate) fn provider_failure_class(&self) -> Option<ProviderFailureClass> {
        match self {
            Self::SttProvider { class, .. } => Some(*class),
            _ => None,
        }
    }

    pub(crate) fn retry_disposition(&self) -> RetryDisposition {
        match self {
            Self::SttNetwork {
                retry_disposition, ..
            } => *retry_disposition,
            Self::SttProvider {
                class: ProviderFailureClass::RateLimited | ProviderFailureClass::ServiceUnavailable,
                ..
            } => RetryDisposition::Retryable,
            _ => RetryDisposition::Terminal,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Audio { message } => message.clone(),
            Self::Config { message } => message.clone(),
            Self::ConfigIo { message } => message.clone(),
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
            Self::SttProvider { message, .. } => (*message).to_string(),
            Self::SttBackpressure { message } => message.clone(),
            Self::SttNetwork { message, .. } => message.clone(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl Error for AppError {}

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

        assert_eq!(value["code"], "osc.send_failed");
        assert_eq!(
            value["message"],
            "Failed to send OSC Chatbox message to 127.0.0.1:9000: network unreachable"
        );
    }

    #[test]
    fn network_error_serializes_with_actionable_stt_code() {
        let error = AppError::stt_network(
            "Could not reach OpenAI. Check your network connection or system proxy settings.",
        );
        let value = serde_json::to_value(&error).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

        assert_eq!(value["code"], "stt.network_unreachable");
        assert!(
            value["message"]
                .as_str()
                .unwrap_or_default()
                .contains("system proxy")
        );
    }

    #[test]
    fn network_retryability_must_be_selected_explicitly() {
        let terminal = AppError::stt_network("A proxy or TLS configuration is invalid.");
        let retryable = AppError::stt_network_retryable("The connection was reset.");

        assert_eq!(terminal.retry_disposition(), RetryDisposition::Terminal);
        assert_eq!(retryable.retry_disposition(), RetryDisposition::Retryable);
        assert_eq!(terminal.code(), "stt.network_unreachable");
        assert_eq!(retryable.code(), "stt.network_unreachable");
    }

    #[test]
    fn backpressure_error_serializes_with_actionable_stt_code() {
        let error = AppError::stt_backpressure(
            "The recognition backend could not keep up with microphone audio; the session stopped instead of silently dropping audio.",
        );
        let value = serde_json::to_value(&error).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

        assert_eq!(value["code"], "stt.backpressure");
        assert!(
            value["message"]
                .as_str()
                .unwrap_or_default()
                .contains("stopped instead of silently dropping audio")
        );
    }
}
