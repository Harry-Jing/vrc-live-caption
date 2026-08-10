//! Tauri-managed application state.
//!
//! Holds the in-memory copy of the non-secret app config plus the runtime
//! manager. Saved-setting and credential operations are serialized with Start
//! here so their persisted and in-memory views cannot drift apart. Plaintext
//! secrets may be resolved transiently for Start, but are never stored in state
//! or in a frontend-facing snapshot.

use crate::capability_planner::{ResolvedPublicationPolicy, RuntimePlanSnapshot, plan_runtime};
use crate::caption_session::{CaptionSessionSnapshotV1, CaptionSessionStore};
use crate::chatbox::ChatboxPacer;
use crate::config::{AppConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, RuntimeStatusEvent, emit_diagnostic,
    emit_runtime_control_and_status,
};
use crate::host_resolver::HostResolver;
use crate::runtime::{RuntimeManager, RuntimeStartOutcome, RuntimeStartRequest};
use crate::runtime_control::{
    RUNTIME_CONTROL_CONTRACT_VERSION, RuntimeControlSnapshot, RuntimeDesiredSnapshot,
    RuntimeSessionPhase, RuntimeSessionSnapshot, pending_session_changes,
};
use crate::saved_settings::{self, SavedSettingsLoad};
use crate::secrets::{
    ProviderSecretStatus, delete_provider_secret, provider_secret_statuses, resolve_openai_api_key,
    save_provider_secret,
};
use std::sync::Mutex;
use tauri::{AppHandle, Runtime};

pub(crate) struct AppState {
    // The only authority for every frontend-visible control field. A snapshot
    // is cloned entirely under this one short lock, which prevents revisions
    // from being paired with state from another instant.
    control: Mutex<RuntimeControlState>,
    // Serializes desired-state mutations with Start's configuration and
    // credential capture. Stop never waits for this gate: file or credential
    // store I/O must not delay the hard generation boundary.
    desired_state_gate: Mutex<()>,
    chatbox_pacer: ChatboxPacer,
    caption_session: CaptionSessionStore,
    host_resolver: HostResolver,
    pub(crate) runtime: RuntimeManager,
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

impl Default for AppState {
    fn default() -> Self {
        Self {
            control: Mutex::new(RuntimeControlState::default()),
            desired_state_gate: Mutex::new(()),
            chatbox_pacer: ChatboxPacer::default(),
            caption_session: CaptionSessionStore::default(),
            host_resolver: HostResolver::default(),
            runtime: RuntimeManager::default(),
        }
    }
}

impl AppState {
    pub(crate) fn start_runtime(&self, app: &AppHandle) -> AppResult<RuntimeControlSnapshot> {
        // Capture before waiting on desired-state I/O. Any later Stop changes
        // the epoch, so this invocation cannot install a runtime after Stop
        // returned while Start was blocked on config or credential work.
        let expected_stop_epoch = self.runtime.stop_epoch();
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;

        if !self.runtime.stop_epoch_unchanged(expected_stop_epoch) {
            return self.runtime_control_snapshot();
        }

        // Reject an active generation before touching the credential store.
        // A second Start must remain an idempotency error even if the key used
        // by the active session was deleted after that session began.
        self.runtime.prepare_for_start(app)?;

        let (config, config_requires_review, config_revision, credential_revision) = {
            let control = self.lock_control()?;
            (
                control.config.clone(),
                control.config_requires_review,
                control.config_revision,
                control.credential_revision,
            )
        };
        if let Err(error) = ensure_config_was_reviewed(config_requires_review) {
            return self.finish_start_failure(app, error, None, expected_stop_epoch);
        }
        if let Err(error) = config.validate() {
            return self.finish_start_failure(app, error, None, expected_stop_epoch);
        }
        let runtime_plan = plan_runtime(&config);
        if let Err(error) = ensure_runtime_plan_is_startable(&runtime_plan) {
            return self.finish_start_failure(app, error, None, expected_stop_epoch);
        }
        let resolved = match resolve_openai_api_key() {
            Ok(resolved) => resolved,
            Err(error) => {
                emit_diagnostic(
                    app,
                    DiagnosticUpdate::error(
                        DiagnosticCategory::Config,
                        "config.openai_api_key_missing",
                        "Cloud STT is not configured",
                        error.to_string(),
                    ),
                );
                return self.finish_start_failure(app, error, None, expected_stop_epoch);
            }
        };
        let credential = crate::runtime_control::RuntimeCredentialSnapshot {
            provider: SttProvider::OpenAi,
            storage: resolved.storage,
            display_suffix: resolved.display_suffix,
            revision: credential_revision,
        };

        let generation = {
            let mut control = self.lock_control()?;
            control.next_generation = control.next_generation.saturating_add(1);
            control.next_generation
        };

        let start_result = self.runtime.start(
            app.clone(),
            RuntimeStartRequest {
                config,
                chatbox_pacer: self.chatbox_pacer(),
                caption_session: self.caption_session_store(),
                host_resolver: self.host_resolver(),
                generation_id: generation,
                config_revision,
                openai_api_key: resolved.secret,
                credential,
                expected_stop_epoch,
            },
            |session| self.install_starting_session(session),
        );

        match start_result {
            Ok(RuntimeStartOutcome::Started | RuntimeStartOutcome::SupersededByStop) => {
                self.runtime_control_snapshot()
            }
            Err(error) => {
                self.finish_start_failure(app, error, Some(generation), expected_stop_epoch)
            }
        }
    }

    pub(crate) fn stop_runtime<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> AppResult<RuntimeControlSnapshot> {
        // Deliberately do not hold the control-state lock while RuntimeManager
        // joins: the worker may emit its final status through that short lock.
        self.runtime.stop(app)?;
        self.runtime_control_snapshot()
    }

    pub(crate) fn runtime_control_snapshot(&self) -> AppResult<RuntimeControlSnapshot> {
        let control = self.lock_control()?;
        Ok(Self::snapshot_from(&control))
    }

    pub(crate) fn caption_session_snapshot(&self) -> AppResult<CaptionSessionSnapshotV1> {
        self.caption_session.snapshot()
    }

    pub(crate) fn caption_session_store(&self) -> CaptionSessionStore {
        self.caption_session.clone()
    }

    pub(crate) fn record_runtime_status(
        &self,
        status: RuntimeStatusEvent,
    ) -> AppResult<RuntimeControlSnapshot> {
        let mut control = self.lock_control()?;
        control.runtime = status.clone();
        match status.status {
            RuntimeStatus::Idle | RuntimeStatus::Stopped => control.session = None,
            RuntimeStatus::Starting => {
                Self::set_session_phase(&mut control, RuntimeSessionPhase::Starting)
            }
            RuntimeStatus::Running => {
                Self::set_session_phase(&mut control, RuntimeSessionPhase::Running)
            }
            RuntimeStatus::Reconnecting => {
                Self::set_session_phase(&mut control, RuntimeSessionPhase::Reconnecting)
            }
            RuntimeStatus::Stopping => {
                Self::set_session_phase(&mut control, RuntimeSessionPhase::Stopping)
            }
            RuntimeStatus::Error => {
                Self::set_session_phase(&mut control, RuntimeSessionPhase::Error)
            }
        }
        Self::advance_revision(&mut control);
        Ok(Self::snapshot_from(&control))
    }

    fn lock_control(&self) -> AppResult<std::sync::MutexGuard<'_, RuntimeControlState>> {
        self.control
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

    fn install_starting_session(&self, session: RuntimeSessionSnapshot) -> AppResult<()> {
        let mut control = self.lock_control()?;
        control.runtime = RuntimeStatusEvent::new(
            RuntimeStatus::Starting,
            Some("Starting outgoing caption runtime".to_string()),
        );
        control.session = Some(session);
        Self::advance_revision(&mut control);
        Ok(())
    }

    #[cfg(test)]
    fn record_start_error(
        &self,
        error: &AppError,
        installed_generation: Option<u64>,
    ) -> AppResult<RuntimeControlSnapshot> {
        let mut control = self.lock_control()?;
        Ok(Self::apply_start_error(
            &mut control,
            error,
            installed_generation,
        ))
    }

    fn record_start_error_if_current(
        &self,
        error: &AppError,
        installed_generation: Option<u64>,
        expected_stop_epoch: u64,
    ) -> AppResult<Option<RuntimeControlSnapshot>> {
        let mut control = self.lock_control()?;

        // Stop advances the epoch before it waits for the runtime handle. Read
        // it while holding the control lock so either this Error is committed
        // first and Stop overwrites it, or the Stop intent wins and this older
        // Start cannot overwrite Stopping/Stopped afterward.
        if !self.runtime.stop_epoch_unchanged(expected_stop_epoch) {
            return Ok(None);
        }

        Ok(Some(Self::apply_start_error(
            &mut control,
            error,
            installed_generation,
        )))
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

    fn finish_start_failure<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        error: AppError,
        installed_generation: Option<u64>,
        expected_stop_epoch: u64,
    ) -> AppResult<RuntimeControlSnapshot> {
        match self.record_start_error_if_current(
            &error,
            installed_generation,
            expected_stop_epoch,
        )? {
            Some(snapshot) => {
                emit_runtime_control_and_status(app, snapshot);
                Err(error)
            }
            None => self.runtime_control_snapshot(),
        }
    }

    fn set_session_phase(control: &mut RuntimeControlState, phase: RuntimeSessionPhase) {
        if let Some(session) = control.session.as_mut() {
            session.phase = phase;
        }
    }

    fn advance_revision(control: &mut RuntimeControlState) {
        control.revision = control.revision.saturating_add(1);
    }

    pub(crate) fn chatbox_pacer(&self) -> ChatboxPacer {
        self.chatbox_pacer.clone()
    }

    pub(crate) fn host_resolver(&self) -> HostResolver {
        self.host_resolver.clone()
    }

    pub(crate) fn osc_config_for_test_message(&self) -> AppResult<crate::config::OscConfig> {
        let control = self.lock_control()?;
        Ok(control
            .session
            .as_ref()
            .map(|session| session.selected.osc.clone())
            .unwrap_or_else(|| control.config.osc.clone()))
    }

    pub(crate) fn load_config<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<AppConfig> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        let (config, config_requires_review) = match saved_settings::load(app)? {
            // A corrupt or invalid config file must not lock the user out of
            // the Settings page (the form only renders with a loaded config),
            // so fall back to defaults and report it; the next save replaces
            // the broken file.
            SavedSettingsLoad::Ready(config) => (config, false),
            SavedSettingsLoad::DefaultsRequireReview {
                config,
                path,
                error,
            } => {
                tracing::warn!(
                    path = %path.display(),
                    error_message = %error,
                    "config file is unusable; defaults loaded"
                );

                emit_diagnostic(
                    app,
                    DiagnosticUpdate::error(
                        DiagnosticCategory::Config,
                        "config.defaults_loaded",
                        "Saved settings could not be loaded",
                        format!(
                            "The settings file at {} is unusable: {error} Default settings are in \
                             use; saving settings replaces the file.",
                            path.display()
                        ),
                    ),
                );

                (config, true)
            }
        };

        let provider_secrets = provider_secret_statuses();
        let mut control = self.lock_control()?;
        control.config = config.clone();
        control.config_requires_review = config_requires_review;
        control.provider_secrets = provider_secrets;
        control.config_revision = control.config_revision.saturating_add(1);
        Self::advance_revision(&mut control);

        Ok(config)
    }

    pub(crate) fn save_config(
        &self,
        app: &AppHandle,
        config: AppConfig,
    ) -> AppResult<RuntimeControlSnapshot> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        saved_settings::save(app, &config)?;

        let mut control = self.lock_control()?;
        control.config = config;
        control.config_requires_review = false;
        control.config_revision = control.config_revision.saturating_add(1);
        Self::advance_revision(&mut control);
        Ok(Self::snapshot_from(&control))
    }

    pub(crate) fn save_provider_secret(
        &self,
        provider: SttProvider,
        secret: String,
    ) -> AppResult<RuntimeControlSnapshot> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        save_provider_secret(provider, secret)?;
        let statuses = provider_secret_statuses();
        let mut control = self.lock_control()?;
        control.credential_revision = control.credential_revision.saturating_add(1);
        control.provider_secrets = statuses;
        Self::advance_revision(&mut control);
        Ok(Self::snapshot_from(&control))
    }

    pub(crate) fn delete_provider_secret(
        &self,
        provider: SttProvider,
    ) -> AppResult<RuntimeControlSnapshot> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        delete_provider_secret(provider)?;
        let statuses = provider_secret_statuses();
        let mut control = self.lock_control()?;
        control.credential_revision = control.credential_revision.saturating_add(1);
        control.provider_secrets = statuses;
        Self::advance_revision(&mut control);
        Ok(Self::snapshot_from(&control))
    }
}

fn ensure_runtime_plan_is_startable(plan: &RuntimePlanSnapshot) -> AppResult<()> {
    match plan.publication.resolved_policy() {
        Some(ResolvedPublicationPolicy::Completed | ResolvedPublicationPolicy::LiveUnit { .. }) => {
            Ok(())
        }
        None => Err(AppError::config(format!(
            "The selected recognition path and publication mode are incompatible ({}).",
            plan.publication
                .incompatibility_code()
                .unwrap_or("publication.incompatible")
        ))),
    }
}

fn ensure_config_was_reviewed(config_requires_review: bool) -> AppResult<()> {
    if config_requires_review {
        Err(AppError::config(
            "Saved settings use a removed or invalid configuration. Review and save the current settings before starting recognition.",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
