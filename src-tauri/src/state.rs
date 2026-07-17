//! Tauri-managed application state.
//!
//! Holds the in-memory copy of the non-secret app config plus the runtime
//! manager. Config reads and writes go through this module so the persisted
//! `config.json` and the in-memory copy cannot drift apart. Plaintext secrets
//! may be resolved transiently for Start, but are never stored in state or in
//! a frontend-facing snapshot.

use crate::chatbox_pacer::ChatboxPacer;
use crate::config::{AppConfig, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatus, RuntimeStatusEvent, emit_diagnostic,
    emit_recorded_status,
};
use crate::runtime::{RuntimeManager, RuntimeStartOutcome, RuntimeStartRequest};
use crate::runtime_control::{
    RUNTIME_CONTROL_CONTRACT_VERSION, RuntimeControlSnapshot, RuntimeDesiredSnapshot,
    RuntimeSessionPhase, RuntimeSessionSnapshot, pending_session_changes,
};
use crate::secrets::{
    ProviderSecretStatus, delete_provider_secret, openai_api_key, provider_secret_statuses,
    save_provider_secret,
};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

const CONFIG_FILE_NAME: &str = "config.json";

pub(crate) struct AppState {
    // The only authority for every frontend-visible control field. A snapshot
    // is cloned entirely under this one short lock, which prevents revisions
    // from being paired with state from another instant.
    control: Mutex<RuntimeControlState>,
    // Serializes desired-state mutations with Start's configuration and
    // credential capture. Stop never waits for this gate: file or credential
    // store I/O must not delay the hard generation boundary.
    desired_state_gate: Mutex<()>,
    // Linearizes the short synthetic-Mock event sequence with Stop. No file,
    // credential, network, pacing, or runtime join work other than Stop itself
    // may be added under this gate.
    runtime_action_gate: Mutex<()>,
    chatbox_pacer: ChatboxPacer,
    pub(crate) runtime: RuntimeManager,
}

struct RuntimeControlState {
    revision: u64,
    config_revision: u64,
    credential_revision: u64,
    next_generation: u64,
    config: AppConfig,
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
            runtime_action_gate: Mutex::new(()),
            chatbox_pacer: ChatboxPacer::default(),
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

        if !self.runtime.start_epoch_is_current(expected_stop_epoch) {
            return self.runtime_control_snapshot();
        }

        // Reject an active generation before touching the credential store.
        // A second Start must remain an idempotency error even if the key used
        // by the active session was deleted after that session began.
        self.runtime.ensure_start_available(app)?;

        let (config, config_revision, credential_revision) = {
            let control = self.lock_control()?;
            (
                control.config.clone(),
                control.config_revision,
                control.credential_revision,
            )
        };
        if let Err(error) = config.validate() {
            return self.finish_start_failure(app, error, None, expected_stop_epoch);
        }
        let (openai_api_key, credential) = if matches!(config.stt.provider, SttProvider::OpenAi) {
            let resolved = match openai_api_key() {
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

            (Some(resolved.secret), Some(credential))
        } else {
            (None, None)
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
                generation_id: generation,
                config_revision,
                openai_api_key,
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
        let _runtime_action = self
            .runtime_action_gate
            .lock()
            .map_err(|_| AppError::state("Runtime action gate was poisoned."))?;
        // Deliberately do not hold the control-state lock while RuntimeManager
        // joins: the worker may emit its final status through that short lock.
        self.runtime.stop(app)?;
        self.runtime_control_snapshot()
    }

    pub(crate) fn runtime_control_snapshot(&self) -> AppResult<RuntimeControlSnapshot> {
        let control = self.lock_control()?;
        Ok(Self::snapshot_from(&control))
    }

    pub(crate) fn session_snapshot(&self) -> AppResult<Option<RuntimeSessionSnapshot>> {
        let control = self.lock_control()?;
        Ok(control.session.clone())
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
        if !self.runtime.start_epoch_is_current(expected_stop_epoch) {
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
                emit_recorded_status(app, snapshot);
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

    pub(crate) fn osc_config_for_test(&self) -> AppResult<crate::config::OscConfig> {
        let control = self.lock_control()?;
        Ok(control
            .session
            .as_ref()
            .map(|session| session.selected.osc.clone())
            .unwrap_or_else(|| control.config.osc.clone()))
    }

    pub(crate) fn with_running_mock_session<T>(
        &self,
        operation: impl FnOnce(&RuntimeSessionSnapshot) -> AppResult<T>,
    ) -> AppResult<T> {
        let _runtime_action = self
            .runtime_action_gate
            .lock()
            .map_err(|_| AppError::state("Runtime action gate was poisoned."))?;
        let session = self
            .session_snapshot()?
            .filter(|session| {
                session.phase == RuntimeSessionPhase::Running
                    && matches!(session.selected.stt.provider, SttProvider::Mock)
            })
            .ok_or_else(|| {
                AppError::runtime("Mock Transcript requires a running Mock provider session.")
            })?;

        operation(&session)
    }

    pub(crate) fn load_config<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<AppConfig> {
        let _operation = self
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        let path = config_path(app)?;
        let config = match fs::read_to_string(&path) {
            // A corrupt or invalid config file must not lock the user out of
            // the Settings page (the form only renders with a loaded config),
            // so fall back to defaults and report it; the next save replaces
            // the broken file.
            Ok(contents) => match parse_valid_config(&contents) {
                Ok(config) => config,
                Err(error) => {
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
                                "The settings file at {} is unusable: {error} Default settings \
                                 are in use; saving settings replaces the file.",
                                path.display()
                            ),
                        ),
                    );

                    AppConfig::default()
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => AppConfig::default(),
            Err(error) => {
                return Err(AppError::config_io(format!(
                    "Failed to read app config at {}: {error}",
                    path.display()
                )));
            }
        };

        let provider_secrets = provider_secret_statuses();
        let mut control = self.lock_control()?;
        control.config = config.clone();
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
        config.validate()?;
        let path = config_path(app)?;
        let parent = path
            .parent()
            .ok_or_else(|| AppError::config_io("App config path has no parent directory."))?;

        fs::create_dir_all(parent).map_err(|error| {
            AppError::config_io(format!(
                "Failed to create app config directory at {}: {error}",
                parent.display()
            ))
        })?;

        let contents = serde_json::to_string_pretty(&config)
            .map_err(|error| AppError::config_io(format!("Failed to serialize config: {error}")))?;

        // Write-then-rename keeps the existing config intact if the app dies
        // mid-write; a torn config.json would otherwise hit load_config's
        // defaults fallback and silently shelve the user's settings.
        let temp_path = path.with_extension("json.tmp");

        fs::write(&temp_path, contents).map_err(|error| {
            AppError::config_io(format!(
                "Failed to write app config at {}: {error}",
                temp_path.display()
            ))
        })?;
        fs::rename(&temp_path, &path).map_err(|error| {
            AppError::config_io(format!(
                "Failed to replace app config at {}: {error}",
                path.display()
            ))
        })?;

        let mut control = self.lock_control()?;
        control.config = config;
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
        if matches!(provider, SttProvider::OpenAi) {
            control.credential_revision = control.credential_revision.saturating_add(1);
        }
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
        if matches!(provider, SttProvider::OpenAi) {
            control.credential_revision = control.credential_revision.saturating_add(1);
        }
        control.provider_secrets = statuses;
        Self::advance_revision(&mut control);
        Ok(Self::snapshot_from(&control))
    }
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> AppResult<PathBuf> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(CONFIG_FILE_NAME))
        .map_err(|error| {
            AppError::config_io(format!("Failed to resolve app config directory: {error}"))
        })
}

fn parse_valid_config(contents: &str) -> AppResult<AppConfig> {
    let config = serde_json::from_str::<AppConfig>(contents)
        .map_err(|error| AppError::config_io(format!("Failed to parse app config: {error}.")))?;

    config.validate()?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_control::{RuntimeChatboxSnapshot, RuntimeSelectedConfig};
    use crate::secrets::ProviderSecretStorage;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;
    use tauri::Listener;

    fn mock_session(config: &AppConfig, generation: u64) -> RuntimeSessionSnapshot {
        RuntimeSessionSnapshot {
            generation,
            phase: RuntimeSessionPhase::Starting,
            started_from_config_revision: 0,
            selected: RuntimeSelectedConfig::from(config),
            credential: None,
            chatbox: RuntimeChatboxSnapshot::Disabled {
                host: config.osc.host.clone(),
                port: config.osc.port,
            },
            uploads_microphone_audio: false,
        }
    }

    #[test]
    fn default_config_passes_validation() -> AppResult<()> {
        AppConfig::default().validate()
    }

    #[test]
    fn runtime_control_snapshot_has_a_versioned_authoritative_shape() -> AppResult<()> {
        let state = AppState::default();
        let snapshot = state.runtime_control_snapshot()?;
        let value = serde_json::to_value(snapshot)
            .map_err(|error| AppError::state(format!("Failed to serialize snapshot: {error}")))?;

        assert_eq!(value["contractVersion"], serde_json::json!(1));
        assert_eq!(value["revision"], serde_json::json!(0));
        assert_eq!(value["desired"]["revision"], serde_json::json!(0));
        assert_eq!(
            value["desired"]["config"]["schemaVersion"],
            serde_json::json!(1)
        );
        assert!(value["session"].is_null());
        assert_eq!(value["pendingChanges"], serde_json::json!([]));

        Ok(())
    }

    #[test]
    fn snapshot_reads_the_cached_desired_secret_status() -> AppResult<()> {
        let state = AppState::default();
        {
            let mut control = state.lock_control()?;
            control.provider_secrets = vec![ProviderSecretStatus {
                provider: "openai".to_string(),
                configured: true,
                storage: Some(ProviderSecretStorage::Environment),
                display_suffix: Some("test".to_string()),
                error: None,
            }];
        }

        let snapshot = state.runtime_control_snapshot()?;
        assert_eq!(
            snapshot.desired.provider_secrets[0]
                .display_suffix
                .as_deref(),
            Some("test")
        );
        Ok(())
    }

    #[test]
    fn snapshot_reads_cannot_mix_a_revision_with_another_config() -> AppResult<()> {
        let state = Arc::new(AppState::default());
        let barrier = Arc::new(Barrier::new(2));
        let writer_state = Arc::clone(&state);
        let writer_barrier = Arc::clone(&barrier);
        let writer = thread::spawn(move || -> AppResult<()> {
            writer_barrier.wait();
            for revision in 1..=2_000_u64 {
                let mut control = writer_state.lock_control()?;
                control.revision = revision;
                control.config_revision = revision;
                control.config.stt.language = format!("revision-{revision}");
            }
            Ok(())
        });

        barrier.wait();
        for _ in 0..2_000 {
            let snapshot = state.runtime_control_snapshot()?;
            if snapshot.revision > 0 {
                assert_eq!(snapshot.desired.revision, snapshot.revision);
                assert_eq!(
                    snapshot.desired.config.stt.language,
                    format!("revision-{}", snapshot.revision)
                );
            }
        }

        writer
            .join()
            .map_err(|_| AppError::runtime("Snapshot writer test thread panicked."))??;
        Ok(())
    }

    #[test]
    fn runtime_error_preserves_the_effective_session_but_stopped_clears_it() -> AppResult<()> {
        let state = AppState::default();
        let mut selected = AppConfig::default();
        selected.stt.provider = SttProvider::Mock;
        state.install_starting_session(mock_session(&selected, 7))?;

        let error_snapshot = state.record_runtime_status(RuntimeStatusEvent::new(
            RuntimeStatus::Error,
            Some("test failure".to_string()),
        ))?;
        assert_eq!(
            error_snapshot.session.as_ref().map(|session| session.phase),
            Some(RuntimeSessionPhase::Error)
        );

        let stopped_snapshot = state.record_runtime_status(RuntimeStatusEvent::new(
            RuntimeStatus::Stopped,
            Some("stopped".to_string()),
        ))?;
        assert!(stopped_snapshot.session.is_none());
        Ok(())
    }

    #[test]
    fn failed_new_start_clears_an_old_error_session() -> AppResult<()> {
        let state = AppState::default();
        let mut selected = AppConfig::default();
        selected.stt.provider = SttProvider::Mock;
        let mut old_session = mock_session(&selected, 11);
        old_session.phase = RuntimeSessionPhase::Error;
        state.install_starting_session(old_session)?;

        let snapshot =
            state.record_start_error(&AppError::secret("OpenAI API key is missing."), None)?;

        assert_eq!(snapshot.runtime.status, RuntimeStatus::Error);
        assert!(snapshot.session.is_none());
        Ok(())
    }

    #[test]
    fn stop_epoch_prevents_a_late_start_error_from_overwriting_stopped() -> AppResult<()> {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
        let state = app.state::<AppState>();
        let expected_stop_epoch = state.runtime.stop_epoch();

        state.runtime.stop(app.handle())?;
        let recorded = state.record_start_error_if_current(
            &AppError::runtime("Late failure from a cancelled Start."),
            None,
            expected_stop_epoch,
        )?;
        let snapshot = state.runtime_control_snapshot()?;

        assert!(recorded.is_none());
        assert_eq!(snapshot.runtime.status, RuntimeStatus::Stopped);
        assert!(snapshot.session.is_none());
        Ok(())
    }

    #[test]
    fn recorded_start_error_publishes_control_before_legacy_status() -> AppResult<()> {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
        let (event_sender, event_receiver) = std::sync::mpsc::channel();
        let control_sender = event_sender.clone();
        app.listen("runtime-control-changed", move |event| {
            let _ = control_sender.send(("control", event.payload().to_string()));
        });
        app.listen("runtime-status", move |event| {
            let _ = event_sender.send(("status", event.payload().to_string()));
        });
        let snapshot = app
            .state::<AppState>()
            .record_start_error(&AppError::config("Invalid test configuration."), None)?;

        emit_recorded_status(app.handle(), snapshot);

        let (first_kind, first_payload) = event_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Start error control event was not delivered."))?;
        let (second_kind, second_payload) = event_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Start error status event was not delivered."))?;
        let control =
            serde_json::from_str::<serde_json::Value>(&first_payload).map_err(|error| {
                AppError::runtime(format!(
                    "Failed to parse start error control event: {error}"
                ))
            })?;
        let status =
            serde_json::from_str::<serde_json::Value>(&second_payload).map_err(|error| {
                AppError::runtime(format!("Failed to parse start error status event: {error}"))
            })?;

        assert_eq!(first_kind, "control");
        assert_eq!(second_kind, "status");
        assert_eq!(control["runtime"]["status"], "error");
        assert!(control["session"].is_null());
        assert_eq!(status["status"], "error");
        Ok(())
    }

    #[test]
    fn thread_spawn_failure_preserves_the_session_it_already_installed() -> AppResult<()> {
        let state = AppState::default();
        let mut selected = AppConfig::default();
        selected.stt.provider = SttProvider::Mock;
        state.install_starting_session(mock_session(&selected, 12))?;

        let snapshot = state.record_start_error(
            &AppError::runtime("Runtime thread could not start."),
            Some(12),
        )?;

        assert_eq!(
            snapshot.session.as_ref().map(|session| session.phase),
            Some(RuntimeSessionPhase::Error)
        );
        Ok(())
    }

    #[test]
    fn mock_operation_uses_running_session_metadata_not_new_desired_settings() -> AppResult<()> {
        let state = AppState::default();
        let mut selected = AppConfig::default();
        selected.stt.provider = SttProvider::Mock;
        selected.stt.language = "ja".to_string();
        state.install_starting_session(mock_session(&selected, 3))?;
        state.record_runtime_status(RuntimeStatusEvent::new(
            RuntimeStatus::Running,
            Some("running".to_string()),
        ))?;
        {
            let mut control = state.lock_control()?;
            control.config.stt.language = "zh".to_string();
        }

        let effective_language =
            state.with_running_mock_session(|session| Ok(session.selected.stt.language.clone()))?;
        assert_eq!(effective_language, "ja");
        Ok(())
    }

    #[test]
    fn osc_test_keeps_using_an_error_sessions_selected_target() -> AppResult<()> {
        let state = AppState::default();
        let mut selected = AppConfig::default();
        selected.stt.provider = SttProvider::Mock;
        selected.osc.host = "192.0.2.10".to_string();
        selected.osc.port = 9010;
        let mut session = mock_session(&selected, 4);
        session.phase = RuntimeSessionPhase::Error;
        state.install_starting_session(session)?;
        {
            let mut control = state.lock_control()?;
            control.config.osc.host = "198.51.100.20".to_string();
            control.config.osc.port = 9020;
        }

        let effective = state.osc_config_for_test()?;
        assert_eq!(effective.host, "192.0.2.10");
        assert_eq!(effective.port, 9010);
        Ok(())
    }

    #[test]
    fn stop_does_not_hold_the_control_lock_while_status_events_clear_session() -> AppResult<()> {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
        let state = app.state::<AppState>();
        let mut selected = AppConfig::default();
        selected.stt.provider = SttProvider::Mock;
        state.install_starting_session(mock_session(&selected, 5))?;
        state.record_runtime_status(RuntimeStatusEvent::new(
            RuntimeStatus::Running,
            Some("running".to_string()),
        ))?;

        let stop_handle = app.handle().clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let result = stop_handle.state::<AppState>().stop_runtime(&stop_handle);
            let _ = sender.send(result);
        });
        let snapshot = receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Stop deadlocked while publishing status."))??;
        worker
            .join()
            .map_err(|_| AppError::runtime("Stop test thread panicked."))?;

        assert_eq!(snapshot.runtime.status, RuntimeStatus::Stopped);
        assert!(snapshot.session.is_none());
        Ok(())
    }

    #[test]
    fn stop_is_not_blocked_by_a_desired_state_operation() -> AppResult<()> {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
        let state = app.state::<AppState>();
        let blocked_operation = state
            .desired_state_gate
            .lock()
            .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
        let stop_handle = app.handle().clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let result = stop_handle.state::<AppState>().stop_runtime(&stop_handle);
            let _ = sender.send(result);
        });

        let prompt_result = receiver.recv_timeout(Duration::from_millis(100));
        drop(blocked_operation);
        let (completed_promptly, stop_result) = match prompt_result {
            Ok(result) => (true, result),
            Err(_) => (
                false,
                receiver.recv_timeout(Duration::from_secs(1)).map_err(|_| {
                    AppError::runtime("Stop did not finish after the desired-state gate opened.")
                })?,
            ),
        };
        worker
            .join()
            .map_err(|_| AppError::runtime("Stop priority test thread panicked."))?;
        let snapshot = stop_result?;

        assert!(
            completed_promptly,
            "Stop waited for an unrelated desired-state operation."
        );
        assert_eq!(snapshot.runtime.status, RuntimeStatus::Stopped);
        Ok(())
    }

    #[test]
    fn default_config_serializes_schema_version() -> Result<(), serde_json::Error> {
        let value = serde_json::to_value(AppConfig::default())?;

        assert_eq!(value.get("schemaVersion"), Some(&serde_json::json!(1)));
        assert!(value.pointer("/osc/minIntervalMs").is_none());

        Ok(())
    }

    #[test]
    fn parse_valid_config_fills_missing_fields_with_defaults() -> AppResult<()> {
        let config = parse_valid_config(r#"{"stt":{"language":"ja"}}"#)?;

        assert_eq!(config.schema_version, 1);
        assert_eq!(config.stt.language, "ja");
        assert!(!config.stt.model.is_empty());

        Ok(())
    }

    #[test]
    fn parse_valid_config_preserves_runtime_settings() -> AppResult<()> {
        let config = parse_valid_config(
            r#"{"audio":{"inputDeviceId":"saved-device"},"osc":{"enabled":false}}"#,
        )?;

        assert_eq!(
            config.audio.input_device_id.as_deref(),
            Some("saved-device")
        );
        assert!(!config.osc.enabled);

        Ok(())
    }

    #[test]
    fn parse_valid_config_ignores_removed_chatbox_interval_and_preserves_other_settings()
    -> AppResult<()> {
        let config = parse_valid_config(
            r#"{
                "schemaVersion": 1,
                "audio": {"inputDeviceId": "saved-device"},
                "stt": {"provider": "mock", "language": "zh", "model": "saved-model"},
                "osc": {
                    "host": "192.0.2.10",
                    "port": 9012,
                    "enabled": false,
                    "minIntervalMs": 750
                },
                "ui": {"showPartial": false}
            }"#,
        )?;

        assert_eq!(config.schema_version, 1);
        assert_eq!(
            config.audio.input_device_id.as_deref(),
            Some("saved-device")
        );
        assert!(matches!(
            config.stt.provider,
            crate::config::SttProvider::Mock
        ));
        assert_eq!(config.stt.language, "zh");
        assert_eq!(config.stt.model, "saved-model");
        assert_eq!(config.osc.host, "192.0.2.10");
        assert_eq!(config.osc.port, 9012);
        assert!(!config.osc.enabled);
        assert!(!config.ui.show_partial);

        Ok(())
    }

    #[test]
    fn parse_valid_config_rejects_malformed_json() {
        assert!(parse_valid_config("{ not json").is_err());
    }

    #[test]
    fn parse_valid_config_rejects_invalid_settings() {
        assert!(parse_valid_config(r#"{"stt":{"language":"  "}}"#).is_err());
    }

    #[test]
    fn parse_valid_config_rejects_unknown_schema_version() {
        assert!(parse_valid_config(r#"{"schemaVersion":2}"#).is_err());
    }
}
