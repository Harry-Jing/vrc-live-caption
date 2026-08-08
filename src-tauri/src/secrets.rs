//! User credential handling for provider API keys.
//!
//! Secrets are stored in the operating system credential store when available.
//! The frontend can save, delete, and inspect status, but it cannot read back
//! plaintext secrets. Runtime code retrieves secrets internally when needed.

use crate::config::SttProvider;
use crate::error::{AppError, AppResult};
use keyring_core::{Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use std::env;
use zeroize::Zeroizing;

const SECRET_SERVICE: &str = "io.github.harry-jing.vrc-live-caption";
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
const OPENAI_ACCOUNT: &str = "provider/openai/default/api-key";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProviderSecretStorage {
    SystemCredentialStore,
    Environment,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderSecretStatus {
    pub(crate) provider: String,
    pub(crate) configured: bool,
    pub(crate) storage: Option<ProviderSecretStorage>,
    pub(crate) display_suffix: Option<String>,
    pub(crate) error: Option<String>,
}

pub(crate) struct ResolvedProviderSecret {
    pub(crate) secret: SecretString,
    pub(crate) storage: ProviderSecretStorage,
    pub(crate) display_suffix: Option<String>,
}

impl ProviderSecretStatus {
    fn unconfigured(provider: SttProvider) -> Self {
        Self {
            provider: provider.as_str().to_string(),
            configured: false,
            storage: None,
            display_suffix: None,
            error: None,
        }
    }

    #[cfg(not(test))]
    fn configured(
        provider: SttProvider,
        storage: ProviderSecretStorage,
        secret: &SecretString,
    ) -> Self {
        Self {
            provider: provider.as_str().to_string(),
            configured: true,
            storage: Some(storage),
            display_suffix: display_suffix(secret.expose_secret()),
            error: None,
        }
    }
}

#[cfg(not(test))]
fn provider_secret_status(provider: SttProvider) -> ProviderSecretStatus {
    match provider {
        SttProvider::OpenAi => openai_secret_status(),
    }
}

#[cfg(not(test))]
pub(crate) fn provider_secret_statuses() -> Vec<ProviderSecretStatus> {
    vec![provider_secret_status(SttProvider::OpenAi)]
}

#[cfg(test)]
pub(crate) fn provider_secret_statuses() -> Vec<ProviderSecretStatus> {
    // Unit tests must not open an operator's real credential store or trigger
    // an OS authorization prompt. Production builds use the implementation
    // above; credential-store behavior is isolated behind the secrets module.
    vec![ProviderSecretStatus::unconfigured(SttProvider::OpenAi)]
}

pub(crate) fn save_provider_secret(provider: SttProvider, secret: String) -> AppResult<()> {
    let secret = Zeroizing::new(secret);
    let secret = normalize_secret(secret.as_str())?;

    match provider {
        SttProvider::OpenAi => save_system_secret(OPENAI_ACCOUNT, &secret),
    }
}

pub(crate) fn delete_provider_secret(provider: SttProvider) -> AppResult<()> {
    match provider {
        SttProvider::OpenAi => delete_system_secret(OPENAI_ACCOUNT),
    }
}

pub(crate) fn openai_api_key() -> AppResult<ResolvedProviderSecret> {
    match read_system_secret(OPENAI_ACCOUNT) {
        Ok(Some(secret)) => Ok(resolved_secret(
            secret,
            ProviderSecretStorage::SystemCredentialStore,
        )),
        Ok(None) => environment_openai_secret()
            .map(|secret| resolved_secret(secret, ProviderSecretStorage::Environment))
            .ok_or_else(missing_openai_api_key),
        Err(error) => environment_openai_secret()
            .map(|secret| resolved_secret(secret, ProviderSecretStorage::Environment))
            .ok_or(error),
    }
}

fn resolved_secret(secret: SecretString, storage: ProviderSecretStorage) -> ResolvedProviderSecret {
    let display_suffix = display_suffix(secret.expose_secret());

    ResolvedProviderSecret {
        secret,
        storage,
        display_suffix,
    }
}

#[cfg(not(test))]
fn openai_secret_status() -> ProviderSecretStatus {
    match read_system_secret(OPENAI_ACCOUNT) {
        Ok(Some(secret)) => ProviderSecretStatus::configured(
            SttProvider::OpenAi,
            ProviderSecretStorage::SystemCredentialStore,
            &secret,
        ),
        Ok(None) => environment_openai_secret()
            .map(|secret| {
                ProviderSecretStatus::configured(
                    SttProvider::OpenAi,
                    ProviderSecretStorage::Environment,
                    &secret,
                )
            })
            .unwrap_or_else(|| ProviderSecretStatus::unconfigured(SttProvider::OpenAi)),
        Err(error) => environment_openai_secret()
            .map(|secret| {
                ProviderSecretStatus::configured(
                    SttProvider::OpenAi,
                    ProviderSecretStorage::Environment,
                    &secret,
                )
            })
            .unwrap_or_else(|| ProviderSecretStatus {
                provider: SttProvider::OpenAi.as_str().to_string(),
                configured: false,
                storage: None,
                display_suffix: None,
                error: Some(error.to_string()),
            }),
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
        "OpenAI API key is not saved. Add it in Settings or set {OPENAI_API_KEY_ENV} before starting cloud STT."
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
        .build(SECRET_SERVICE, account, None)
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
    fn test_secret_statuses_expose_only_openai_without_opening_the_real_store() {
        let statuses = provider_secret_statuses();
        assert_eq!(statuses.len(), 1);
        let status = &statuses[0];
        assert_eq!(status.provider, "openai");
        assert!(!status.configured);
        assert!(status.storage.is_none());
        assert!(status.error.is_none());
    }
}
