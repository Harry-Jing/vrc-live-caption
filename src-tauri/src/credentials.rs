//! User credential handling for external-service API keys.
//!
//! Secrets are stored in the operating system credential store when available.
//! The frontend can save, delete, and inspect status, but it cannot read back
//! plaintext secrets. During Start, the desktop composition boundary resolves a
//! plaintext secret and binds it inside the prepared Recognition Module. Runtime
//! control state and frontend-facing snapshots expose only non-secret metadata.

use crate::error::{AppError, AppResult};
use keyring_core::{Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::env;
use zeroize::Zeroizing;

const KEYRING_SERVICE_ID: &str = "io.github.harry-jing.vrc-live-caption";
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
// This persisted account identifier predates the service-credential vocabulary.
// Keep it stable so upgrades continue to find users' existing API keys.
const OPENAI_ACCOUNT: &str = "provider/openai/default/api-key";
const CUSTOM_TRANSLATION_ACCOUNT: &str = "translation/custom/default/api-key";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CredentialStorage {
    SystemCredentialStore,
    Environment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CredentialId {
    #[serde(rename = "openai")]
    OpenAi,
    CustomTranslation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum CredentialStatus {
    Unconfigured {
        id: CredentialId,
    },
    Configured {
        id: CredentialId,
        storage: CredentialStorage,
        display_suffix: Option<String>,
    },
    Unavailable {
        id: CredentialId,
        failure: CredentialFailure,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialFailure {
    pub(crate) code: String,
    pub(crate) message: String,
}

pub(crate) struct ResolvedCredential {
    pub(crate) secret: SecretString,
    pub(crate) storage: CredentialStorage,
    pub(crate) display_suffix: Option<String>,
}

impl CredentialStatus {
    fn unconfigured(id: CredentialId) -> Self {
        Self::Unconfigured { id }
    }

    #[cfg(not(test))]
    fn configured(id: CredentialId, storage: CredentialStorage, secret: &SecretString) -> Self {
        Self::Configured {
            id,
            storage,
            display_suffix: display_suffix(secret.expose_secret()),
        }
    }

    fn unavailable(id: CredentialId, error: &AppError) -> Self {
        Self::Unavailable {
            id,
            failure: CredentialFailure {
                code: error.code().to_string(),
                message: error.to_string(),
            },
        }
    }
}

#[cfg(not(test))]
fn credential_status(id: CredentialId) -> CredentialStatus {
    match id {
        CredentialId::OpenAi => openai_credential_status(),
        CredentialId::CustomTranslation => custom_translation_credential_status(),
    }
}

#[cfg(not(test))]
pub(crate) fn credential_statuses() -> Vec<CredentialStatus> {
    vec![
        credential_status(CredentialId::OpenAi),
        credential_status(CredentialId::CustomTranslation),
    ]
}

#[cfg(test)]
pub(crate) fn credential_statuses() -> Vec<CredentialStatus> {
    // Unit tests must not open an operator's real credential store or trigger
    // an OS authorization prompt. Production builds use the implementation
    // above; credential-store behavior is isolated behind the credentials module.
    vec![
        CredentialStatus::unconfigured(CredentialId::OpenAi),
        CredentialStatus::unconfigured(CredentialId::CustomTranslation),
    ]
}

pub(crate) fn save_credential(id: CredentialId, secret: String) -> AppResult<()> {
    let secret = Zeroizing::new(secret);
    let secret = normalize_secret(secret.as_str())?;

    match id {
        CredentialId::OpenAi => save_system_secret(OPENAI_ACCOUNT, &secret),
        CredentialId::CustomTranslation => save_system_secret(CUSTOM_TRANSLATION_ACCOUNT, &secret),
    }
}

pub(crate) fn delete_credential(id: CredentialId) -> AppResult<()> {
    match id {
        CredentialId::OpenAi => delete_system_secret(OPENAI_ACCOUNT),
        CredentialId::CustomTranslation => delete_system_secret(CUSTOM_TRANSLATION_ACCOUNT),
    }
}

pub(crate) fn resolve_openai_credential() -> AppResult<ResolvedCredential> {
    match read_system_secret(OPENAI_ACCOUNT) {
        Ok(Some(secret)) => Ok(resolved_credential(
            secret,
            CredentialStorage::SystemCredentialStore,
        )),
        Ok(None) => environment_openai_secret()
            .map(|secret| resolved_credential(secret, CredentialStorage::Environment))
            .ok_or_else(missing_openai_api_key),
        Err(error) => environment_openai_secret()
            .map(|secret| resolved_credential(secret, CredentialStorage::Environment))
            .ok_or(error),
    }
}

fn resolved_credential(secret: SecretString, storage: CredentialStorage) -> ResolvedCredential {
    let display_suffix = display_suffix(secret.expose_secret());

    ResolvedCredential {
        secret,
        storage,
        display_suffix,
    }
}

#[cfg(not(test))]
fn openai_credential_status() -> CredentialStatus {
    match read_system_secret(OPENAI_ACCOUNT) {
        Ok(Some(secret)) => CredentialStatus::configured(
            CredentialId::OpenAi,
            CredentialStorage::SystemCredentialStore,
            &secret,
        ),
        Ok(None) => environment_openai_secret()
            .map(|secret| {
                CredentialStatus::configured(
                    CredentialId::OpenAi,
                    CredentialStorage::Environment,
                    &secret,
                )
            })
            .unwrap_or_else(|| CredentialStatus::unconfigured(CredentialId::OpenAi)),
        Err(error) => environment_openai_secret()
            .map(|secret| {
                CredentialStatus::configured(
                    CredentialId::OpenAi,
                    CredentialStorage::Environment,
                    &secret,
                )
            })
            .unwrap_or_else(|| CredentialStatus::unavailable(CredentialId::OpenAi, &error)),
    }
}

#[cfg(not(test))]
fn custom_translation_credential_status() -> CredentialStatus {
    match read_system_secret(CUSTOM_TRANSLATION_ACCOUNT) {
        Ok(Some(secret)) => CredentialStatus::configured(
            CredentialId::CustomTranslation,
            CredentialStorage::SystemCredentialStore,
            &secret,
        ),
        Ok(None) => CredentialStatus::unconfigured(CredentialId::CustomTranslation),
        Err(error) => CredentialStatus::unavailable(CredentialId::CustomTranslation, &error),
    }
}

fn environment_openai_secret() -> Option<SecretString> {
    env::var(OPENAI_API_KEY_ENV).ok().and_then(|value| {
        let value = Zeroizing::new(value);

        normalize_secret(value.as_str()).ok()
    })
}

fn missing_openai_api_key() -> AppError {
    AppError::secret(format!(
        "OpenAI API key is not saved. Add it in Settings or set {OPENAI_API_KEY_ENV} before starting cloud recognition."
    ))
}

fn normalize_secret(secret: &str) -> AppResult<SecretString> {
    let secret = secret.trim();

    if secret.is_empty() {
        return Err(AppError::secret("API key cannot be empty."));
    }

    if secret.chars().any(char::is_control) {
        return Err(AppError::secret(
            "API key cannot contain control characters.",
        ));
    }

    Ok(SecretString::from(secret.to_string()))
}

fn display_suffix(secret: &str) -> Option<String> {
    let suffix: String = secret.chars().rev().take(4).collect();

    if suffix.is_empty() {
        None
    } else {
        Some(suffix.chars().rev().collect())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn read_system_secret(account: &str) -> AppResult<Option<SecretString>> {
    let entry = system_entry(account)?;

    match entry.get_password() {
        Ok(secret) => {
            let secret = Zeroizing::new(secret);

            normalize_secret(secret.as_str()).map(Some)
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(keyring_error("read", error)),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn read_system_secret(_account: &str) -> AppResult<Option<SecretString>> {
    Err(AppError::secret(
        "System credential store is not supported on this platform.",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn save_system_secret(account: &str, secret: &SecretString) -> AppResult<()> {
    let entry = system_entry(account)?;

    entry
        .set_password(secret.expose_secret())
        .map_err(|error| keyring_error("save", error))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn save_system_secret(_account: &str, _secret: &SecretString) -> AppResult<()> {
    Err(AppError::secret(
        "System credential store is not supported on this platform.",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn delete_system_secret(account: &str) -> AppResult<()> {
    let entry = system_entry(account)?;

    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(keyring_error("delete", error)),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn delete_system_secret(_account: &str) -> AppResult<()> {
    Err(AppError::secret(
        "System credential store is not supported on this platform.",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn system_entry(account: &str) -> AppResult<Entry> {
    system_credential_store()
        .map_err(|error| keyring_error("open", error))?
        .build(KEYRING_SERVICE_ID, account, None)
        .map_err(|error| keyring_error("open", error))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn keyring_error(action: &str, error: KeyringError) -> AppError {
    AppError::secret(format!(
        "Failed to {action} API key in the system credential store: {error}"
    ))
}

#[cfg(target_os = "linux")]
fn system_credential_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, KeyringError>
{
    dbus_secret_service_keyring_store::Store::new()
        .map(|store| store as std::sync::Arc<keyring_core::CredentialStore>)
}

#[cfg(target_os = "macos")]
fn system_credential_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, KeyringError>
{
    apple_native_keyring_store::keychain::Store::new()
        .map(|store| store as std::sync::Arc<keyring_core::CredentialStore>)
}

#[cfg(target_os = "windows")]
fn system_credential_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, KeyringError>
{
    windows_native_keyring_store::Store::new()
        .map(|store| store as std::sync::Arc<keyring_core::CredentialStore>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_suffix_uses_at_most_last_four_characters() {
        assert_eq!(display_suffix("sk-test-abcdef"), Some("cdef".to_string()));
        assert_eq!(display_suffix("abc"), Some("abc".to_string()));
        assert_eq!(display_suffix(""), None);
    }

    #[test]
    fn normalize_secret_rejects_empty_or_control_text() {
        assert!(normalize_secret("  ").is_err());
        assert!(normalize_secret("abc\ndef").is_err());
        assert!(normalize_secret(" sk-valid ").is_ok());
    }

    #[test]
    fn test_credential_statuses_expose_both_ids_without_opening_the_real_store() {
        let statuses = credential_statuses();
        assert_eq!(
            statuses,
            vec![
                CredentialStatus::Unconfigured {
                    id: CredentialId::OpenAi,
                },
                CredentialStatus::Unconfigured {
                    id: CredentialId::CustomTranslation,
                },
            ]
        );
    }

    #[test]
    fn custom_translation_credential_id_has_its_own_wire_value() {
        let value = serde_json::to_value(CredentialStatus::unconfigured(
            CredentialId::CustomTranslation,
        ))
        .unwrap_or_else(|error| serde_json::json!({ "serializationError": error.to_string() }));

        assert_eq!(
            value,
            serde_json::json!({
                "state": "unconfigured",
                "id": "customTranslation",
            })
        );
        assert_ne!(CredentialId::CustomTranslation, CredentialId::OpenAi);
        assert_eq!(
            CredentialStatus::unconfigured(CredentialId::OpenAi),
            CredentialStatus::Unconfigured {
                id: CredentialId::OpenAi,
            }
        );
    }

    #[test]
    fn unconfigured_status_serializes_without_configured_only_fields() {
        let value = serde_json::to_value(CredentialStatus::unconfigured(CredentialId::OpenAi))
            .unwrap_or_else(|error| serde_json::json!({ "serializationError": error.to_string() }));

        assert_eq!(
            value,
            serde_json::json!({
                "state": "unconfigured",
                "id": "openai",
            })
        );
    }

    #[test]
    fn unavailable_status_serializes_the_stable_application_failure() {
        let value = serde_json::to_value(CredentialStatus::unavailable(
            CredentialId::OpenAi,
            &AppError::secret("System credential store is unavailable."),
        ))
        .unwrap_or_else(|error| serde_json::json!({ "serializationError": error.to_string() }));

        assert_eq!(
            value,
            serde_json::json!({
                "state": "unavailable",
                "id": "openai",
                "failure": {
                    "code": "config.secret_failed",
                    "message": "System credential store is unavailable.",
                },
            })
        );
    }
}
