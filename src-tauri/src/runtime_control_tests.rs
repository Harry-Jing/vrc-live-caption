use super::*;
use crate::caption::TranslationFailureReason;
use crate::config::{
    ContentSelection, TranslationConfig, TranslationEndpoint, TranslationPath, TranslationTarget,
};
use crate::error::{AppError, AppResult};

fn translation(target: TranslationTarget) -> TranslationConfig {
    TranslationConfig {
        path: TranslationPath::OpenAiResponsesCompletedText,
        target,
        endpoint: TranslationEndpoint::Official,
    }
}

fn config_with_translation(target: TranslationTarget) -> AppConfig {
    AppConfig {
        translation: Some(translation(target)),
        ..Default::default()
    }
}

fn generation_snapshot(config: &AppConfig, generation: u64) -> RuntimeGenerationSnapshot {
    RuntimeGenerationSnapshot {
        id: generation,
        phase: RuntimeGenerationPhase::Starting,
        started_from_config_revision: 0,
        selection: RuntimeGenerationSelection::from(config),
        caption_pipeline_plan: plan_caption_pipeline(config),
        credentials: Vec::new(),
        chatbox_publication: ChatboxPublicationSnapshot::Disabled {
            host: config.osc.host.clone(),
            port: config.osc.port,
        },
        translation_state: if RuntimeGenerationSelection::from(config)
            .translation
            .is_some()
        {
            RuntimeGenerationTranslationState::Active
        } else {
            RuntimeGenerationTranslationState::Inactive
        },
        uploads_microphone_audio: false,
        uploads_source_text: RuntimeGenerationSelection::from(config)
            .translation
            .is_some(),
    }
}

#[test]
fn ui_only_desired_change_does_not_require_generation_restart() {
    let selection = RuntimeGenerationSelection::from(&AppConfig::default());
    let mut desired = AppConfig::default();
    desired.ui.show_ongoing_preview = false;

    assert!(
        pending_generation_changes(&desired, &selection, &CredentialRevisions::default(), &[],)
            .is_empty()
    );
}

#[test]
fn publication_change_requires_a_new_generation_without_becoming_an_osc_change() {
    let selection = RuntimeGenerationSelection::from(&AppConfig::default());
    let mut desired = AppConfig::default();
    desired.publication.mode = crate::config::PublicationMode::Live;

    assert_eq!(
        pending_generation_changes(&desired, &selection, &CredentialRevisions::default(), &[],),
        vec![PendingGenerationChange::Publication]
    );
}

#[test]
fn active_translation_change_requires_a_new_generation() {
    let mut active = config_with_translation(TranslationTarget::English);
    active.publication.content = ContentSelection::TranslationOnly;
    let selection = RuntimeGenerationSelection::from(&active);
    let mut desired = active;
    desired.translation = Some(translation(TranslationTarget::SimplifiedChinese));

    assert_eq!(
        pending_generation_changes(&desired, &selection, &CredentialRevisions::default(), &[],),
        vec![PendingGenerationChange::Translation]
    );
}

#[test]
fn dormant_translation_change_does_not_require_a_new_generation() {
    let active = config_with_translation(TranslationTarget::English);
    let selection = RuntimeGenerationSelection::from(&active);
    let mut desired = active;
    desired.translation = Some(translation(TranslationTarget::SimplifiedChinese));

    assert!(
        pending_generation_changes(&desired, &selection, &CredentialRevisions::default(), &[],)
            .is_empty()
    );
}

#[test]
fn activating_translation_content_is_only_a_publication_change() {
    let active = config_with_translation(TranslationTarget::English);
    let selection = RuntimeGenerationSelection::from(&active);
    let mut desired = active;
    desired.translation = Some(translation(TranslationTarget::SimplifiedChinese));
    desired.publication.content = ContentSelection::Bilingual;

    assert_eq!(
        pending_generation_changes(&desired, &selection, &CredentialRevisions::default(), &[],),
        vec![PendingGenerationChange::Publication]
    );
}

#[test]
fn openai_credential_change_requires_a_new_generation() {
    let active = AppConfig::default();
    let selection = RuntimeGenerationSelection::from(&active);
    let mut revisions = CredentialRevisions::default();
    revisions.advance(CredentialId::OpenAi);
    revisions.advance(CredentialId::OpenAi);

    assert_eq!(
        pending_generation_changes(
            &active,
            &selection,
            &revisions,
            &[RuntimeGenerationCredentialSnapshot {
                id: CredentialId::OpenAi,
                storage: CredentialStorage::Environment,
                display_suffix: None,
                revision: 1,
            }],
        ),
        vec![PendingGenerationChange::Credential]
    );
}

#[test]
fn credential_changes_do_not_restart_a_generation_without_a_credential() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    store.install_starting_generation(generation_snapshot(&selected, 1))?;
    store.replace_credential_statuses(
        CredentialId::OpenAi,
        vec![CredentialStatus::Configured {
            id: CredentialId::OpenAi,
            storage: CredentialStorage::Environment,
            display_suffix: Some("updated".to_string()),
        }],
    )?;

    assert!(
        !store
            .snapshot()?
            .pending_generation_changes
            .contains(&PendingGenerationChange::Credential)
    );
    Ok(())
}

#[test]
fn changing_unused_custom_translation_credential_does_not_restart_source_only() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = config_with_translation(TranslationTarget::SimplifiedChinese);
    let mut generation = generation_snapshot(&selected, 1);
    generation.credentials = vec![RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::Environment,
        display_suffix: Some("used".to_string()),
        revision: 0,
    }];
    store.install_starting_generation(generation)?;
    store.replace_credential_statuses(
        CredentialId::CustomTranslation,
        vec![CredentialStatus::Configured {
            id: CredentialId::CustomTranslation,
            storage: CredentialStorage::SystemCredentialStore,
            display_suffix: Some("new".to_string()),
        }],
    )?;

    assert!(
        !store
            .snapshot()?
            .pending_generation_changes
            .contains(&PendingGenerationChange::Credential)
    );
    Ok(())
}

#[test]
fn changing_used_custom_translation_credential_requires_a_new_generation() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let mut selected = AppConfig {
        translation: Some(TranslationConfig {
            path: TranslationPath::OpenAiResponsesCompletedText,
            target: TranslationTarget::SimplifiedChinese,
            endpoint: TranslationEndpoint::Custom {
                api_base_url: crate::config::ApiBaseUrl::parse("https://example.com/v1")
                    .map_err(AppError::config)?,
            },
        }),
        ..Default::default()
    };
    selected.publication.content = ContentSelection::TranslationOnly;
    store.replace_loaded_config(selected.clone(), false, Vec::new())?;
    let mut generation = generation_snapshot(&selected, 1);
    generation.credentials = vec![
        RuntimeGenerationCredentialSnapshot {
            id: CredentialId::OpenAi,
            storage: CredentialStorage::Environment,
            display_suffix: Some("openai".to_string()),
            revision: 0,
        },
        RuntimeGenerationCredentialSnapshot {
            id: CredentialId::CustomTranslation,
            storage: CredentialStorage::SystemCredentialStore,
            display_suffix: Some("custom".to_string()),
            revision: 0,
        },
    ];
    store.install_starting_generation(generation)?;

    let snapshot =
        store.replace_credential_statuses(CredentialId::CustomTranslation, Vec::new())?;

    assert_eq!(
        snapshot.pending_generation_changes,
        vec![PendingGenerationChange::Credential]
    );
    Ok(())
}

#[test]
fn chatbox_snapshot_uses_the_shared_host_and_port_wire_names() {
    let value = serde_json::to_value(ChatboxPublicationSnapshot::Unavailable {
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
        serde_json::json!(2)
    );
    assert_eq!(
        value["desired"]["captionPipelinePlan"]["publication"]["state"],
        serde_json::json!("compatible")
    );
    assert!(value["generation"].is_null());
    assert_eq!(value["pendingGenerationChanges"], serde_json::json!([]));

    Ok(())
}

#[test]
fn source_only_generation_exposes_translation_as_inactive() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = config_with_translation(TranslationTarget::SimplifiedChinese);
    store.install_starting_generation(generation_snapshot(&selected, 20))?;

    let generation = store
        .snapshot()?
        .generation
        .ok_or_else(|| AppError::state("Source-only generation was not installed."))?;
    assert!(matches!(
        generation.translation_state,
        RuntimeGenerationTranslationState::Inactive
    ));
    assert!(generation.selection.translation.is_none());
    assert!(!generation.uploads_source_text);
    Ok(())
}

#[test]
fn translation_degradation_is_generation_scoped_and_sticky() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let mut selected = config_with_translation(TranslationTarget::SimplifiedChinese);
    selected.publication.content = ContentSelection::Bilingual;
    store.install_starting_generation(generation_snapshot(&selected, 20))?;
    let old = store.translation_status_recorder(20);
    store.install_starting_generation(generation_snapshot(&selected, 21))?;
    let current = store.translation_status_recorder(21);
    let before = store.snapshot()?.revision;

    assert!(
        old.record_degraded(TranslationFailureReason::Failed)?
            .is_none()
    );
    assert_eq!(store.snapshot()?.revision, before);
    let degraded = current
        .record_degraded(TranslationFailureReason::ProviderUnavailable)?
        .ok_or_else(|| AppError::state("Current generation was not degraded."))?;
    assert_eq!(degraded.revision, before + 1);
    assert_eq!(degraded.runtime_status.status, RuntimeStatus::Starting);
    assert!(matches!(
        degraded
            .generation
            .as_ref()
            .map(|generation| &generation.translation_state),
        Some(RuntimeGenerationTranslationState::Degraded {
            reason_code: TranslationFailureReason::ProviderUnavailable,
        })
    ));
    assert!(
        current
            .record_degraded(TranslationFailureReason::DeadlineExceeded)?
            .is_none()
    );
    assert_eq!(store.snapshot()?.revision, degraded.revision);
    Ok(())
}

#[test]
fn inactive_translation_cannot_be_degraded() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    store.install_starting_generation(generation_snapshot(&selected, 22))?;
    let before = store.snapshot()?;

    assert!(
        store
            .translation_status_recorder(22)
            .record_degraded(TranslationFailureReason::ProviderUnavailable)?
            .is_none()
    );
    let after = store.snapshot()?;
    assert_eq!(after.revision, before.revision);
    assert!(matches!(
        after
            .generation
            .as_ref()
            .map(|generation| &generation.translation_state),
        Some(RuntimeGenerationTranslationState::Inactive)
    ));
    Ok(())
}

#[test]
fn snapshot_reads_the_cached_desired_credential_status() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    store.replace_loaded_config(
        AppConfig::default(),
        false,
        vec![CredentialStatus::Configured {
            id: CredentialId::OpenAi,
            storage: CredentialStorage::Environment,
            display_suffix: Some("test".to_string()),
        }],
    )?;

    let snapshot = store.snapshot()?;
    assert_eq!(
        snapshot.desired.credentials,
        vec![CredentialStatus::Configured {
            id: CredentialId::OpenAi,
            storage: CredentialStorage::Environment,
            display_suffix: Some("test".to_string()),
        }]
    );
    Ok(())
}

#[test]
fn saved_config_snapshot_keeps_its_desired_revision_and_config_together() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let mut config = AppConfig::default();
    config.recognition.expected_languages = vec!["zh".to_string(), "en".to_string()];

    let snapshot = store.replace_saved_config(config.clone())?;

    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.desired.revision, 1);
    assert_eq!(snapshot.desired.config, config);
    Ok(())
}

#[test]
fn runtime_error_preserves_the_effective_generation_but_stopped_clears_it() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    store.install_starting_generation(generation_snapshot(&selected, 7))?;
    let recorder = store.status_recorder();

    let error_snapshot = recorder.record(RuntimeStatusEvent::new(
        RuntimeStatus::Error,
        Some("test failure".to_string()),
    ))?;
    assert_eq!(
        error_snapshot
            .generation
            .as_ref()
            .map(|generation| generation.phase),
        Some(RuntimeGenerationPhase::Error)
    );

    let stopped_snapshot = recorder.record(RuntimeStatusEvent::new(
        RuntimeStatus::Stopped,
        Some("stopped".to_string()),
    ))?;
    assert!(stopped_snapshot.generation.is_none());
    Ok(())
}

#[test]
fn reconnecting_status_keeps_the_effective_generation_active() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    store.install_starting_generation(generation_snapshot(&selected, 8))?;

    let snapshot = store.status_recorder().record(RuntimeStatusEvent::new(
        RuntimeStatus::Reconnecting,
        Some("Reconnecting speech recognition".to_string()),
    ))?;

    assert_eq!(snapshot.runtime_status.status, RuntimeStatus::Reconnecting);
    assert_eq!(
        snapshot
            .generation
            .as_ref()
            .map(|generation| generation.phase),
        Some(RuntimeGenerationPhase::Reconnecting)
    );
    Ok(())
}

#[test]
fn failed_new_start_clears_an_old_error_generation() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    let mut old_generation = generation_snapshot(&selected, 11);
    old_generation.phase = RuntimeGenerationPhase::Error;
    store.install_starting_generation(old_generation)?;

    let snapshot =
        store.record_start_error(&AppError::secret("OpenAI API key is missing."), None)?;

    assert_eq!(snapshot.runtime_status.status, RuntimeStatus::Error);
    assert!(snapshot.generation.is_none());
    Ok(())
}

#[test]
fn current_generation_start_failure_preserves_the_installed_selection() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let selected = AppConfig::default();
    store.install_starting_generation(generation_snapshot(&selected, 12))?;

    let snapshot = store.record_start_error(
        &AppError::runtime("Runtime thread could not start."),
        Some(12),
    )?;

    assert_eq!(
        snapshot
            .generation
            .as_ref()
            .map(|generation| generation.phase),
        Some(RuntimeGenerationPhase::Error)
    );
    Ok(())
}

#[test]
fn osc_test_keeps_using_an_error_generations_selected_target() -> AppResult<()> {
    let store = RuntimeControlStore::default();
    let mut selected = AppConfig::default();
    selected.osc.host = "192.0.2.10".to_string();
    selected.osc.port = 9010;
    let mut generation = generation_snapshot(&selected, 4);
    generation.phase = RuntimeGenerationPhase::Error;
    store.install_starting_generation(generation)?;
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
        assert_eq!(
            control.credential_revisions.revision(CredentialId::OpenAi),
            0
        );
    }

    store.replace_credential_statuses(CredentialId::OpenAi, Vec::new())?;
    {
        let control = store.lock()?;
        assert_eq!(control.revision, 2);
        assert_eq!(control.config_revision, 1);
        assert_eq!(
            control.credential_revisions.revision(CredentialId::OpenAi),
            1
        );
        assert_eq!(
            control
                .credential_revisions
                .revision(CredentialId::CustomTranslation),
            0
        );
    }

    store.replace_saved_config(AppConfig::default())?;
    let control = store.lock()?;
    assert_eq!(control.revision, 3);
    assert_eq!(control.config_revision, 2);
    assert_eq!(
        control.credential_revisions.revision(CredentialId::OpenAi),
        1
    );
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
