//! Authoritative desired settings and effective runtime-session state.

use crate::capability_planner::{RuntimePlanSnapshot, plan_runtime};
use crate::config::{AppConfig, AudioConfig, OscConfig, PublicationConfig, SttConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::secrets::{ProviderSecretStatus, ProviderSecretStorage};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const RUNTIME_CONTROL_CONTRACT_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeStatusEvent {
    pub(crate) status: RuntimeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    pub(crate) timestamp_ms: u64,
}

impl RuntimeStatusEvent {
    pub(crate) fn idle() -> Self {
        Self::new(RuntimeStatus::Idle, Some("Runtime is idle".to_string()))
    }

    pub(crate) fn new(status: RuntimeStatus, message: Option<String>) -> Self {
        Self {
            status,
            message,
            timestamp_ms: runtime_status_now_ms(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeStatus {
    Idle,
    Starting,
    Running,
    Reconnecting,
    Stopping,
    Stopped,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeControlSnapshot {
    pub(crate) contract_version: u32,
    pub(crate) revision: u64,
    pub(crate) runtime: RuntimeStatusEvent,
    pub(crate) desired: RuntimeDesiredSnapshot,
    pub(crate) session: Option<RuntimeSessionSnapshot>,
    pub(crate) pending_changes: Vec<PendingSessionChange>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeDesiredSnapshot {
    pub(crate) revision: u64,
    pub(crate) config: AppConfig,
    pub(crate) runtime_plan: RuntimePlanSnapshot,
    pub(crate) provider_secrets: Vec<ProviderSecretStatus>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSessionSnapshot {
    pub(crate) generation: u64,
    pub(crate) phase: RuntimeSessionPhase,
    pub(crate) started_from_config_revision: u64,
    pub(crate) selected: RuntimeSelectedConfig,
    pub(crate) runtime_plan: RuntimePlanSnapshot,
    pub(crate) credential: Option<RuntimeCredentialSnapshot>,
    pub(crate) chatbox: RuntimeChatboxSnapshot,
    pub(crate) uploads_microphone_audio: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeSessionPhase {
    Starting,
    Running,
    Reconnecting,
    Stopping,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeCredentialSnapshot {
    pub(crate) provider: SttProvider,
    pub(crate) storage: ProviderSecretStorage,
    pub(crate) display_suffix: Option<String>,
    pub(crate) revision: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum RuntimeChatboxSnapshot {
    Disabled {
        host: String,
        port: u16,
    },
    Ready {
        host: String,
        port: u16,
    },
    Unavailable {
        host: String,
        port: u16,
        #[serde(rename = "reasonCode")]
        reason_code: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSelectedConfig {
    pub(crate) audio: AudioConfig,
    pub(crate) stt: SttConfig,
    pub(crate) osc: OscConfig,
    pub(crate) publication: PublicationConfig,
}

impl From<&AppConfig> for RuntimeSelectedConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            audio: config.audio.clone(),
            stt: config.stt.clone(),
            osc: config.osc.clone(),
            publication: config.publication.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PendingSessionChange {
    Microphone,
    Recognition,
    Credential,
    ChatboxOutput,
    Publication,
}

fn pending_session_changes(
    desired: &AppConfig,
    selected: &RuntimeSelectedConfig,
    desired_credential_revision: u64,
    session_credential_revision: u64,
) -> Vec<PendingSessionChange> {
    let mut changes = Vec::new();

    if desired.audio != selected.audio {
        changes.push(PendingSessionChange::Microphone);
    }
    if desired.stt != selected.stt {
        changes.push(PendingSessionChange::Recognition);
    }
    if desired_credential_revision != session_credential_revision {
        changes.push(PendingSessionChange::Credential);
    }
    if desired.osc != selected.osc {
        changes.push(PendingSessionChange::ChatboxOutput);
    }
    if desired.publication != selected.publication {
        changes.push(PendingSessionChange::Publication);
    }

    changes
}

/// Cloneable authority for every field in [`RuntimeControlSnapshot`].
///
/// The inner state is intentionally private so revisions and their associated
/// values can only be observed or changed atomically through domain-specific
/// operations.
#[derive(Clone)]
pub(crate) struct RuntimeControlStore {
    inner: Arc<Mutex<RuntimeControlState>>,
}

struct RuntimeControlState {
    revision: u64,
    config_revision: u64,
    credential_revision: u64,
    next_generation: u64,
    config: AppConfig,
    config_requires_review: bool,
    provider_secrets: Vec<ProviderSecretStatus>,
    runtime: RuntimeStatusEvent,
    session: Option<RuntimeSessionSnapshot>,
}

impl Default for RuntimeControlState {
    fn default() -> Self {
        Self {
            revision: 0,
            config_revision: 0,
            credential_revision: 0,
            next_generation: 0,
            config: AppConfig::default(),
            config_requires_review: false,
            provider_secrets: Vec::new(),
            runtime: RuntimeStatusEvent::idle(),
            session: None,
        }
    }
}

impl Default for RuntimeControlStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeControlState::default())),
        }
    }
}

/// The lifecycle-only capability handed to runtime event recording.
///
/// It can advance status and the corresponding session phase, but cannot
/// mutate desired settings, credentials, generations, or start selections.
#[derive(Clone)]
pub(crate) struct RuntimeStatusRecorder {
    inner: Arc<Mutex<RuntimeControlState>>,
}

#[derive(Clone)]
pub(crate) struct RuntimeStartSelection {
    pub(crate) config: AppConfig,
    pub(crate) config_requires_review: bool,
    pub(crate) config_revision: u64,
    pub(crate) credential_revision: u64,
}

impl RuntimeControlStore {
    pub(crate) fn snapshot(&self) -> AppResult<RuntimeControlSnapshot> {
        let control = self.lock()?;
        Ok(Self::snapshot_from(&control))
    }

    pub(crate) fn start_selection(&self) -> AppResult<RuntimeStartSelection> {
        let control = self.lock()?;
        Ok(RuntimeStartSelection {
            config: control.config.clone(),
            config_requires_review: control.config_requires_review,
            config_revision: control.config_revision,
            credential_revision: control.credential_revision,
        })
    }

    pub(crate) fn allocate_generation(&self) -> AppResult<u64> {
        let mut control = self.lock()?;
        control.next_generation = control.next_generation.saturating_add(1);
        Ok(control.next_generation)
    }

    pub(crate) fn install_starting_session(
        &self,
        session: RuntimeSessionSnapshot,
    ) -> AppResult<()> {
        let mut control = self.lock()?;
        control.runtime = RuntimeStatusEvent::new(
            RuntimeStatus::Starting,
            Some("Starting outgoing caption runtime".to_string()),
        );
        control.session = Some(session);
        Self::advance_revision(&mut control);
        Ok(())
    }

    pub(crate) fn record_start_error_if_current(
        &self,
        error: &AppError,
        installed_generation: Option<u64>,
        is_current: impl FnOnce() -> bool,
    ) -> AppResult<Option<RuntimeControlSnapshot>> {
        let mut control = self.lock()?;

        // Evaluate the Stop epoch while holding the control lock. Therefore
        // either Error is committed first and Stop overwrites it, or Stop's
        // intent wins and the older Start cannot overwrite it afterward.
        if !is_current() {
            return Ok(None);
        }

        Ok(Some(Self::apply_start_error(
            &mut control,
            error,
            installed_generation,
        )))
    }

    #[cfg(test)]
    pub(crate) fn record_start_error(
        &self,
        error: &AppError,
        installed_generation: Option<u64>,
    ) -> AppResult<RuntimeControlSnapshot> {
        let mut control = self.lock()?;
        Ok(Self::apply_start_error(
            &mut control,
            error,
            installed_generation,
        ))
    }

    pub(crate) fn effective_osc_config(&self) -> AppResult<OscConfig> {
        let control = self.lock()?;
        Ok(control
            .session
            .as_ref()
            .map(|session| session.selected.osc.clone())
            .unwrap_or_else(|| control.config.osc.clone()))
    }

    pub(crate) fn replace_loaded_config(
        &self,
        config: AppConfig,
        config_requires_review: bool,
        provider_secrets: Vec<ProviderSecretStatus>,
    ) -> AppResult<()> {
        let mut control = self.lock()?;
        control.config = config;
        control.config_requires_review = config_requires_review;
        control.provider_secrets = provider_secrets;
        control.config_revision = control.config_revision.saturating_add(1);
        Self::advance_revision(&mut control);
        Ok(())
    }

    pub(crate) fn replace_saved_config(
        &self,
        config: AppConfig,
    ) -> AppResult<RuntimeControlSnapshot> {
        let mut control = self.lock()?;
        control.config = config;
        control.config_requires_review = false;
        control.config_revision = control.config_revision.saturating_add(1);
        Self::advance_revision(&mut control);
        Ok(Self::snapshot_from(&control))
    }

    pub(crate) fn replace_provider_secret_statuses(
        &self,
        provider_secrets: Vec<ProviderSecretStatus>,
    ) -> AppResult<RuntimeControlSnapshot> {
        let mut control = self.lock()?;
        control.credential_revision = control.credential_revision.saturating_add(1);
        control.provider_secrets = provider_secrets;
        Self::advance_revision(&mut control);
        Ok(Self::snapshot_from(&control))
    }

    pub(crate) fn status_recorder(&self) -> RuntimeStatusRecorder {
        RuntimeStatusRecorder {
            inner: Arc::clone(&self.inner),
        }
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, RuntimeControlState>> {
        self.inner
            .lock()
            .map_err(|_| AppError::state("Runtime control state lock was poisoned."))
    }

    fn snapshot_from(control: &RuntimeControlState) -> RuntimeControlSnapshot {
        let pending_changes = control
            .session
            .as_ref()
            .map(|session| {
                pending_session_changes(
                    &control.config,
                    &session.selected,
                    control.credential_revision,
                    session
                        .credential
                        .as_ref()
                        .map(|credential| credential.revision)
                        .unwrap_or(0),
                )
            })
            .unwrap_or_default();

        RuntimeControlSnapshot {
            contract_version: RUNTIME_CONTROL_CONTRACT_VERSION,
            revision: control.revision,
            runtime: control.runtime.clone(),
            desired: RuntimeDesiredSnapshot {
                revision: control.config_revision,
                config: control.config.clone(),
                runtime_plan: plan_runtime(&control.config),
                provider_secrets: control.provider_secrets.clone(),
            },
            session: control.session.clone(),
            pending_changes,
        }
    }

    fn apply_start_error(
        control: &mut RuntimeControlState,
        error: &AppError,
        installed_generation: Option<u64>,
    ) -> RuntimeControlSnapshot {
        control.runtime = RuntimeStatusEvent::new(RuntimeStatus::Error, Some(error.to_string()));
        if control.session.as_ref().map(|session| session.generation) == installed_generation {
            Self::set_session_phase(control, RuntimeSessionPhase::Error);
        } else {
            control.session = None;
        }
        Self::advance_revision(control);
        Self::snapshot_from(control)
    }

    fn set_session_phase(control: &mut RuntimeControlState, phase: RuntimeSessionPhase) {
        if let Some(session) = control.session.as_mut() {
            session.phase = phase;
        }
    }

    fn advance_revision(control: &mut RuntimeControlState) {
        control.revision = control.revision.saturating_add(1);
    }
}

impl RuntimeStatusRecorder {
    pub(crate) fn record(&self, status: RuntimeStatusEvent) -> AppResult<RuntimeControlSnapshot> {
        let mut control = self
            .inner
            .lock()
            .map_err(|_| AppError::state("Runtime control state lock was poisoned."))?;
        control.runtime = status.clone();
        match status.status {
            RuntimeStatus::Idle | RuntimeStatus::Stopped => control.session = None,
            RuntimeStatus::Starting => {
                RuntimeControlStore::set_session_phase(&mut control, RuntimeSessionPhase::Starting);
            }
            RuntimeStatus::Running => {
                RuntimeControlStore::set_session_phase(&mut control, RuntimeSessionPhase::Running);
            }
            RuntimeStatus::Reconnecting => RuntimeControlStore::set_session_phase(
                &mut control,
                RuntimeSessionPhase::Reconnecting,
            ),
            RuntimeStatus::Stopping => {
                RuntimeControlStore::set_session_phase(&mut control, RuntimeSessionPhase::Stopping);
            }
            RuntimeStatus::Error => {
                RuntimeControlStore::set_session_phase(&mut control, RuntimeSessionPhase::Error);
            }
        }
        RuntimeControlStore::advance_revision(&mut control);
        Ok(RuntimeControlStore::snapshot_from(&control))
    }
}

fn runtime_status_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[cfg(test)]
#[path = "runtime_control_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_control_contract_tests.rs"]
mod contract_tests;
