use super::*;
use crate::error::{AppError, AppResult};
use std::sync::{Arc, Barrier};
use std::thread;

fn session_snapshot(config: &AppConfig, generation: u64) -> RuntimeSessionSnapshot {
    RuntimeSessionSnapshot {
        generation,
        phase: RuntimeSessionPhase::Starting,
        started_from_config_revision: 0,
        selected: RuntimeSelectedConfig::from(config),
        runtime_plan: plan_runtime(config),
        credential: None,
        chatbox: RuntimeChatboxSnapshot::Disabled {
            host: config.osc.host.clone(),
            port: config.osc.port,
        },
        uploads_microphone_audio: false,
    }
}

#[test]
fn ui_only_desired_change_does_not_require_session_restart() {
    let selected = RuntimeSelectedConfig::from(&AppConfig::default());
    let mut desired = AppConfig::default();
    desired.ui.show_partial = false;

    assert!(pending_session_changes(&desired, &selected, 0, 0).is_empty());
}

#[test]
fn publication_change_requires_a_new_session_without_becoming_an_osc_change() {
    let selected = RuntimeSelectedConfig::from(&AppConfig::default());
    let mut desired = AppConfig::default();
    desired.publication.mode = crate::config::PublicationMode::Live;

    assert_eq!(
        pending_session_changes(&desired, &selected, 0, 0),
        vec![PendingSessionChange::Publication]
    );
}

#[test]
fn openai_credential_change_requires_a_new_session() {
    let active = AppConfig::default();
    let selected = RuntimeSelectedConfig::from(&active);

    assert_eq!(
        pending_session_changes(&active, &selected, 2, 1),
        vec![PendingSessionChange::Credential]
    );
}

#[test]
fn chatbox_snapshot_uses_the_shared_host_and_port_wire_names() {
    let value = serde_json::to_value(RuntimeChatboxSnapshot::Unavailable {
        host: "127.0.0.1".to_string(),
        port: 9000,
        reason_code: "osc.bind_failed".to_string(),
    })
    .unwrap_or_else(|error| serde_json::json!({ "serializationError": error.to_string() }));

    assert_eq!(value["state"], "unavailable");
    assert_eq!(value["host"], "127.0.0.1");
    assert_eq!(value["port"], 9000);
    assert_eq!(value["reasonCode"], "osc.bind_failed");
    assert!(value.get("requestedHost").is_none());
    assert!(value.get("requestedPort").is_none());
}

#[test]
fn snapshot_has_a_versioned_authoritative_shape() -> AppResult<()> {
    let snapshot = RuntimeControlStore::default().snapshot()?;
    let value = serde_json::to_value(snapshot)
        .map_err(|error| AppError::state(format!("Failed to serialize snapshot: {error}")))?;

    assert_eq!(value["contractVersion"], serde_json::json!(3));
    assert_eq!(value["revision"], serde_json::json!(0));
    assert_eq!(value["desired"]["revision"], serde_json::json!(0));
    assert_eq!(
        value["desired"]["config"]["schemaVersion"],
        serde_json::json!(3)
    );
    assert_eq!(
        value["desired"]["runtimePlan"]["publication"]["state"],
        serde_json::json!("ready")
    );
    assert!(value["session"].is_null());
    assert_eq!(value["pendingChanges"], serde_json::json!([]));

    Ok(())
}

#[test]
fn snapshot_reads_the_cached_desired_secret_status() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    store.replace_loaded_config(
        AppConfig::default(),
        false,
        vec![ProviderSecretStatus {
            provider: "openai".to_string(),
            configured: true,
            storage: Some(ProviderSecretStorage::Environment),
            display_suffix: Some("test".to_string()),
            error: None,
        }],
    )?;

    let snapshot = store.snapshot()?;
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
    let store = Arc::new(RuntimeControlStore::default());
    let barrier = Arc::new(Barrier::new(2));
    let writer_store = Arc::clone(&store);
    let writer_barrier = Arc::clone(&barrier);
    let writer = thread::spawn(move || -> AppResult<()> {
        writer_barrier.wait();
        for revision in 1..=2_000_u64 {
            let mut control = writer_store.lock()?;
            control.revision = revision;
            control.config_revision = revision;
            control.config.stt.languages = vec![format!("revision-{revision}")];
        }
        Ok(())
    });

    barrier.wait();
    for _ in 0..2_000 {
        let snapshot = store.snapshot()?;
        if snapshot.revision > 0 {
            assert_eq!(snapshot.desired.revision, snapshot.revision);
            assert_eq!(
                snapshot.desired.config.stt.languages,
                vec![format!("revision-{}", snapshot.revision)]
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
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    store.install_starting_session(session_snapshot(&selected, 7))?;
    let recorder = store.status_recorder();

    let error_snapshot = recorder.record(RuntimeStatusEvent::new(
        RuntimeStatus::Error,
        Some("test failure".to_string()),
    ))?;
    assert_eq!(
        error_snapshot.session.as_ref().map(|session| session.phase),
        Some(RuntimeSessionPhase::Error)
    );

    let stopped_snapshot = recorder.record(RuntimeStatusEvent::new(
        RuntimeStatus::Stopped,
        Some("stopped".to_string()),
    ))?;
    assert!(stopped_snapshot.session.is_none());
    Ok(())
}

#[test]
fn reconnecting_status_keeps_the_effective_session_active() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    store.install_starting_session(session_snapshot(&selected, 8))?;

    let snapshot = store.status_recorder().record(RuntimeStatusEvent::new(
        RuntimeStatus::Reconnecting,
        Some("Reconnecting speech recognition".to_string()),
    ))?;

    assert_eq!(snapshot.runtime.status, RuntimeStatus::Reconnecting);
    assert_eq!(
        snapshot.session.as_ref().map(|session| session.phase),
        Some(RuntimeSessionPhase::Reconnecting)
    );
    Ok(())
}

#[test]
fn failed_new_start_clears_an_old_error_session() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    let mut old_session = session_snapshot(&selected, 11);
    old_session.phase = RuntimeSessionPhase::Error;
    store.install_starting_session(old_session)?;

    let snapshot =
        store.record_start_error(&AppError::secret("OpenAI API key is missing."), None)?;

    assert_eq!(snapshot.runtime.status, RuntimeStatus::Error);
    assert!(snapshot.session.is_none());
    Ok(())
}

#[test]
fn current_generation_start_failure_preserves_the_installed_selection() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    store.install_starting_session(session_snapshot(&selected, 12))?;

    let snapshot = store.record_start_error(
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
fn osc_test_keeps_using_an_error_sessions_selected_target() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let mut selected = AppConfig::default();
    selected.osc.host = "192.0.2.10".to_string();
    selected.osc.port = 9010;
    let mut session = session_snapshot(&selected, 4);
    session.phase = RuntimeSessionPhase::Error;
    store.install_starting_session(session)?;
    let mut desired = AppConfig::default();
    desired.osc.host = "198.51.100.20".to_string();
    desired.osc.port = 9020;
    store.replace_saved_config(desired)?;

    let effective = store.effective_osc_config()?;
    assert_eq!(effective.host, "192.0.2.10");
    assert_eq!(effective.port, 9010);
    Ok(())
}

#[test]
fn clones_and_recorders_share_one_authoritative_revision() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let clone = store.clone();
    let recorder = store.status_recorder();

    clone.replace_saved_config(AppConfig::default())?;
    let snapshot = recorder.record(RuntimeStatusEvent::new(RuntimeStatus::Running, None))?;

    assert_eq!(snapshot.revision, 2);
    assert_eq!(store.snapshot()?.revision, 2);
    Ok(())
}

#[test]
fn desired_and_credential_mutations_advance_only_their_owned_revisions() -> AppResult<()> {
    let store = RuntimeControlStore::default();

    store.replace_loaded_config(AppConfig::default(), false, Vec::new())?;
    {
        let control = store.lock()?;
        assert_eq!(control.revision, 1);
        assert_eq!(control.config_revision, 1);
        assert_eq!(control.credential_revision, 0);
    }

    store.replace_provider_secret_statuses(Vec::new())?;
    {
        let control = store.lock()?;
        assert_eq!(control.revision, 2);
        assert_eq!(control.config_revision, 1);
        assert_eq!(control.credential_revision, 1);
    }

    store.replace_saved_config(AppConfig::default())?;
    let control = store.lock()?;
    assert_eq!(control.revision, 3);
    assert_eq!(control.config_revision, 2);
    assert_eq!(control.credential_revision, 1);
    Ok(())
}

#[test]
fn generation_allocation_is_monotonic_without_advancing_the_wire_revision() -> AppResult<()> {
    let store = RuntimeControlStore::default();

    assert_eq!(store.allocate_generation()?, 1);
    assert_eq!(store.allocate_generation()?, 2);
    assert_eq!(store.snapshot()?.revision, 0);
    Ok(())
}
