//! Authoritative desired settings and effective runtime-generation state.

use crate::caption_pipeline::{CaptionPipelinePlanSnapshot, plan_caption_pipeline};
use crate::config::{AppConfig, AudioConfig, OscConfig, PublicationConfig, RecognitionConfig};
use crate::credentials::{CredentialId, CredentialStatus, CredentialStorage};
use crate::error::{AppError, AppResult};
use crate::wall_clock::unix_timestamp_ms;
use serde::Serialize;
use std::sync::{Arc, Mutex};

pub(crate) const RUNTIME_CONTROL_CONTRACT_VERSION: u32 = 4;

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
            timestamp_ms: unix_timestamp_ms(),
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
    pub(crate) runtime_status: RuntimeStatusEvent,
    pub(crate) desired: RuntimeDesiredSnapshot,
    pub(crate) generation: Option<RuntimeGenerationSnapshot>,
    pub(crate) pending_generation_changes: Vec<PendingGenerationChange>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeDesiredSnapshot {
    pub(crate) revision: u64,
    pub(crate) config: AppConfig,
    pub(crate) caption_pipeline_plan: CaptionPipelinePlanSnapshot,
    pub(crate) credentials: Vec<CredentialStatus>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeGenerationSnapshot {
    pub(crate) id: u64,
    pub(crate) phase: RuntimeGenerationPhase,
    pub(crate) started_from_config_revision: u64,
    pub(crate) selection: RuntimeGenerationSelection,
    pub(crate) caption_pipeline_plan: CaptionPipelinePlanSnapshot,
    pub(crate) credential: Option<RuntimeGenerationCredentialSnapshot>,
    pub(crate) chatbox_publication: ChatboxPublicationSnapshot,
    pub(crate) uploads_microphone_audio: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeGenerationPhase {
    Starting,
    Running,
    Reconnecting,
    Stopping,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeGenerationCredentialSnapshot {
    pub(crate) id: CredentialId,
    pub(crate) storage: CredentialStorage,
    pub(crate) display_suffix: Option<String>,
    pub(crate) revision: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum ChatboxPublicationSnapshot {
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
pub(crate) struct RuntimeGenerationSelection {
    pub(crate) audio: AudioConfig,
    pub(crate) recognition: RecognitionConfig,
    pub(crate) osc: OscConfig,
    pub(crate) publication: PublicationConfig,
}

impl From<&AppConfig> for RuntimeGenerationSelection {
    fn from(config: &AppConfig) -> Self {
        // Classify every saved field explicitly as generation-scoped or not.
        // Omitting `..` makes a future AppConfig field a compile-time decision
        // instead of silently presenting desired state as active state.
        let AppConfig {
            schema_version: _,
            audio,
            recognition,
            osc,
            publication,
            ui: _,
        } = config;

        Self {
            audio: audio.clone(),
            recognition: recognition.clone(),
            osc: osc.clone(),
            publication: publication.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PendingGenerationChange {
    Microphone,
    Recognition,
    Credential,
    ChatboxOutput,
    Publication,
}

fn pending_generation_changes(
    desired: &AppConfig,
    selection: &RuntimeGenerationSelection,
    desired_credential_revision: u64,
    generation_credential_revision: Option<u64>,
) -> Vec<PendingGenerationChange> {
    let mut changes = Vec::new();

    if desired.audio != selection.audio {
        changes.push(PendingGenerationChange::Microphone);
    }
    if desired.recognition != selection.recognition {
        changes.push(PendingGenerationChange::Recognition);
    }
    if generation_credential_revision
        .is_some_and(|revision| desired_credential_revision != revision)
    {
        changes.push(PendingGenerationChange::Credential);
    }
    if desired.osc != selection.osc {
        changes.push(PendingGenerationChange::ChatboxOutput);
    }
    if desired.publication != selection.publication {
        changes.push(PendingGenerationChange::Publication);
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
    credentials: Vec<CredentialStatus>,
    runtime_status: RuntimeStatusEvent,
    generation: Option<RuntimeGenerationSnapshot>,
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
            credentials: Vec::new(),
            runtime_status: RuntimeStatusEvent::idle(),
            generation: None,
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
/// It can advance status and the corresponding generation phase, but cannot
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

    pub(crate) fn install_starting_generation(
        &self,
        generation: RuntimeGenerationSnapshot,
    ) -> AppResult<()> {
        let mut control = self.lock()?;
        control.runtime_status = RuntimeStatusEvent::new(
            RuntimeStatus::Starting,
            Some("Starting caption runtime".to_string()),
        );
        control.generation = Some(generation);
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
            .generation
            .as_ref()
            .map(|generation| generation.selection.osc.clone())
            .unwrap_or_else(|| control.config.osc.clone()))
    }

    pub(crate) fn replace_loaded_config(
        &self,
        config: AppConfig,
        config_requires_review: bool,
        credentials: Vec<CredentialStatus>,
    ) -> AppResult<()> {
        let mut control = self.lock()?;
        control.config = config;
        control.config_requires_review = config_requires_review;
        control.credentials = credentials;
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

    pub(crate) fn replace_credential_statuses(
        &self,
        credentials: Vec<CredentialStatus>,
    ) -> AppResult<RuntimeControlSnapshot> {
        let mut control = self.lock()?;
        control.credential_revision = control.credential_revision.saturating_add(1);
        control.credentials = credentials;
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
        let pending_generation_changes = control
            .generation
            .as_ref()
            .map(|generation| {
                pending_generation_changes(
                    &control.config,
                    &generation.selection,
                    control.credential_revision,
                    generation
                        .credential
                        .as_ref()
                        .map(|credential| credential.revision),
                )
            })
            .unwrap_or_default();

        RuntimeControlSnapshot {
            contract_version: RUNTIME_CONTROL_CONTRACT_VERSION,
            revision: control.revision,
            runtime_status: control.runtime_status.clone(),
            desired: RuntimeDesiredSnapshot {
                revision: control.config_revision,
                config: control.config.clone(),
                caption_pipeline_plan: plan_caption_pipeline(&control.config),
                credentials: control.credentials.clone(),
            },
            generation: control.generation.clone(),
            pending_generation_changes,
        }
    }

    fn apply_start_error(
        control: &mut RuntimeControlState,
        error: &AppError,
        installed_generation: Option<u64>,
    ) -> RuntimeControlSnapshot {
        control.runtime_status =
            RuntimeStatusEvent::new(RuntimeStatus::Error, Some(error.to_string()));
        if control.generation.as_ref().map(|generation| generation.id) == installed_generation {
            Self::set_generation_phase(control, RuntimeGenerationPhase::Error);
        } else {
            control.generation = None;
        }
        Self::advance_revision(control);
        Self::snapshot_from(control)
    }

    fn set_generation_phase(control: &mut RuntimeControlState, phase: RuntimeGenerationPhase) {
        if let Some(generation) = control.generation.as_mut() {
            generation.phase = phase;
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
        control.runtime_status = status.clone();
        match status.status {
            RuntimeStatus::Idle | RuntimeStatus::Stopped => control.generation = None,
            RuntimeStatus::Starting => {
                RuntimeControlStore::set_generation_phase(
                    &mut control,
                    RuntimeGenerationPhase::Starting,
                );
            }
            RuntimeStatus::Running => {
                RuntimeControlStore::set_generation_phase(
                    &mut control,
                    RuntimeGenerationPhase::Running,
                );
            }
            RuntimeStatus::Reconnecting => RuntimeControlStore::set_generation_phase(
                &mut control,
                RuntimeGenerationPhase::Reconnecting,
            ),
            RuntimeStatus::Stopping => {
                RuntimeControlStore::set_generation_phase(
                    &mut control,
                    RuntimeGenerationPhase::Stopping,
                );
            }
            RuntimeStatus::Error => {
                RuntimeControlStore::set_generation_phase(
                    &mut control,
                    RuntimeGenerationPhase::Error,
                );
            }
        }
        RuntimeControlStore::advance_revision(&mut control);
        Ok(RuntimeControlStore::snapshot_from(&control))
    }
}

#[cfg(test)]
#[path = "runtime_control_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_control_contract_tests.rs"]
mod contract_tests;
