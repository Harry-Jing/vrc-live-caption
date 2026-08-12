use super::*;
use crate::caption::CaptionAggregateStore;
use crate::config::{
    ApiBaseUrl, ContentSelection, TranslationConfig, TranslationEndpoint, TranslationPath,
    TranslationTarget,
};
use crate::credentials::{CredentialId, CredentialStorage, ResolvedCredential};
use crate::host_resolver::{HostResolutionError, HostResolver};
use crate::recognition::{
    RecognitionDriver, RecognitionDriverIo, openai_gpt_live_transcribe_module,
    openai_gpt_transcribe_module,
};
use crate::runtime::PreparedTranslation;
use crate::runtime_control::RuntimeControlStore;
use secrecy::SecretString;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tauri::Listener;

fn test_recognition_module(config: &AppConfig) -> AppResult<RecognitionModule> {
    match config.recognition.path {
        crate::config::RecognitionPath::OpenAiGptTranscribe => openai_gpt_transcribe_module(
            config.recognition.expected_languages.clone(),
            SecretString::from("test-key".to_string()),
            HostResolver::default(),
        ),
        crate::config::RecognitionPath::OpenAiGptLiveTranscribe => {
            openai_gpt_live_transcribe_module(
                config.recognition.expected_languages.clone(),
                SecretString::from("test-key".to_string()),
                HostResolver::default(),
            )
        }
    }
}

fn official_translation() -> TranslationConfig {
    TranslationConfig {
        path: TranslationPath::OpenAiResponsesCompletedText,
        target: TranslationTarget::SimplifiedChinese,
        endpoint: TranslationEndpoint::Official,
    }
}

fn prepared_translation(
    selection: TranslationConfig,
    credential: RuntimeGenerationCredentialSnapshot,
) -> AppResult<PreparedTranslation> {
    let revision = credential.revision;
    let resolved = ResolvedCredential {
        id: credential.id,
        secret: SecretString::from("test-translation-key".to_string()),
        storage: credential.storage,
        display_suffix: credential.display_suffix,
    };
    let (binding, _) = crate::translation::translation_module_for_test(
        selection,
        resolved,
        revision,
        std::iter::empty(),
    )?;
    Ok(PreparedTranslation::cloud(binding))
}

struct WaitForRuntimeStopDriver {
    started: mpsc::SyncSender<()>,
    completed_runs: Arc<AtomicUsize>,
}

impl RecognitionDriver for WaitForRuntimeStopDriver {
    fn run(self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        self.started
            .send(())
            .map_err(|_| AppError::state("Runtime test dropped its Driver-start receiver."))?;
        io.wait_until_stopped()?;
        self.completed_runs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn runtime_start_request(
    manager: &RuntimeManager,
    config: AppConfig,
    recognition_credential: RuntimeGenerationCredentialSnapshot,
    prepared_translation: Option<PreparedTranslation>,
) -> AppResult<(RuntimeStartRequest, mpsc::Receiver<()>, Arc<AtomicUsize>)> {
    let (driver_started, started) = mpsc::sync_channel(1);
    let completed_runs = Arc::new(AtomicUsize::new(0));
    let recognition_module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        WaitForRuntimeStopDriver {
            started: driver_started,
            completed_runs: Arc::clone(&completed_runs),
        },
    )?;
    Ok((
        RuntimeStartRequest {
            config,
            chatbox_pacer: ChatboxPacer::default(),
            caption_aggregate: CaptionAggregateStore::default(),
            chatbox_host_resolver: HostResolver::default(),
            prepared_recognition: PreparedRecognition::cloud(
                recognition_module,
                recognition_credential,
            )?,
            prepared_translation,
            generation_id: 1,
            config_revision: 2,
            status_recorder: RuntimeControlStore::default().status_recorder(),
            expected_stop_epoch: manager.stop_epoch(),
        },
        started,
        completed_runs,
    ))
}

#[test]
fn cloud_recognition_rejects_non_openai_credential_metadata() -> AppResult<()> {
    let error = PreparedRecognition::cloud(
        test_recognition_module(&AppConfig::default())?,
        RuntimeGenerationCredentialSnapshot {
            id: CredentialId::CustomTranslation,
            storage: CredentialStorage::SystemCredentialStore,
            display_suffix: Some("wrong".to_string()),
            revision: 4,
        },
    )
    .err()
    .ok_or_else(|| AppError::state("Cloud Recognition accepted non-OpenAI metadata."))?;

    assert_eq!(error.code(), "runtime.state_failed");
    Ok(())
}

#[test]
fn prepared_cloud_recognition_binds_generation_disclosure_to_its_module() -> AppResult<()> {
    let credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::Environment,
        display_suffix: Some("test".to_string()),
        revision: 7,
    };
    let prepared = PreparedRecognition::cloud(
        test_recognition_module(&AppConfig::default())?,
        credential.clone(),
    )?;
    let PreparedRecognition {
        module,
        credentials,
        uploads_microphone_audio,
    } = prepared;

    drop(module);
    let prepared_credential = credentials
        .first()
        .ok_or_else(|| AppError::state("Prepared cloud recognition lost its credential."))?;
    assert_eq!(prepared_credential.id, credential.id);
    assert_eq!(prepared_credential.storage, credential.storage);
    assert_eq!(
        prepared_credential.display_suffix,
        credential.display_suffix
    );
    assert_eq!(prepared_credential.revision, credential.revision);
    assert!(uploads_microphone_audio);
    Ok(())
}

#[test]
fn runtime_rejects_a_prepared_translation_for_source_only_selection() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();
    let config = AppConfig::default();
    let credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::Environment,
        display_suffix: Some("same".to_string()),
        revision: 3,
    };
    let request = RuntimeStartRequest {
        prepared_translation: Some(prepared_translation(
            official_translation(),
            credential.clone(),
        )?),
        config: config.clone(),
        chatbox_pacer: ChatboxPacer::default(),
        caption_aggregate: CaptionAggregateStore::default(),
        chatbox_host_resolver: HostResolver::default(),
        prepared_recognition: PreparedRecognition::cloud(
            test_recognition_module(&config)?,
            credential,
        )?,
        generation_id: 1,
        config_revision: 1,
        status_recorder: RuntimeControlStore::default().status_recorder(),
        expected_stop_epoch: manager.stop_epoch(),
    };

    let error = manager
        .start(app.handle().clone(), request, |_| Ok(()))
        .err()
        .ok_or_else(|| AppError::state("Source-only Runtime accepted a Translation owner."))?;

    assert_eq!(error.code(), "runtime.state_failed");
    assert!(error.to_string().contains("Source-only"));
    assert!(
        manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?
            .is_none()
    );
    Ok(())
}

#[test]
fn official_translation_reuses_the_exact_openai_generation_credential() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let manager = RuntimeManager::default();
    let mut config = AppConfig::default();
    config.osc.enabled = false;
    config.translation = Some(official_translation());
    config.publication.content = ContentSelection::TranslationOnly;
    let credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::Environment,
        display_suffix: Some("shared".to_string()),
        revision: 7,
    };
    let (mut request, started, _) = runtime_start_request(
        &manager,
        config,
        credential.clone(),
        Some(prepared_translation(
            official_translation(),
            credential.clone(),
        )?),
    )?;
    request.status_recorder = status_recorder.clone();
    let (snapshot_sender, snapshot_receiver) = mpsc::sync_channel(1);

    assert_eq!(
        manager.start(app.handle().clone(), request, |snapshot| {
            snapshot_sender.send(snapshot).map_err(|_| {
                AppError::state("Runtime test dropped its generation-snapshot receiver.")
            })
        })?,
        RuntimeStartOutcome::Started
    );
    started
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Prepared Recognition Driver did not start."))?;
    let snapshot = snapshot_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Prepared generation snapshot was not installed."))?;

    assert!(matches!(
        snapshot.translation_state,
        RuntimeGenerationTranslationState::Active
    ));
    assert!(snapshot.uploads_source_text);
    assert_eq!(snapshot.credentials.len(), 1);
    let shared = snapshot
        .credentials
        .first()
        .ok_or_else(|| AppError::state("Official Translation lost its shared credential."))?;
    assert_eq!(shared.id, CredentialId::OpenAi);
    assert_eq!(shared.storage, credential.storage);
    assert_eq!(shared.display_suffix, credential.display_suffix);
    assert_eq!(shared.revision, credential.revision);

    manager.stop(app.handle(), &status_recorder)
}

#[test]
fn custom_translation_adds_its_bound_credential_to_the_generation() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let manager = RuntimeManager::default();
    let selection = TranslationConfig {
        path: TranslationPath::OpenAiResponsesCompletedText,
        target: TranslationTarget::English,
        endpoint: TranslationEndpoint::Custom {
            api_base_url: ApiBaseUrl::parse("https://translate.example.test/v1")
                .map_err(AppError::config)?,
        },
    };
    let mut config = AppConfig::default();
    config.osc.enabled = false;
    config.translation = Some(selection.clone());
    config.publication.content = ContentSelection::Bilingual;
    let recognition_credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::Environment,
        display_suffix: Some("openai".to_string()),
        revision: 2,
    };
    let translation_credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::CustomTranslation,
        storage: CredentialStorage::SystemCredentialStore,
        display_suffix: Some("custom".to_string()),
        revision: 9,
    };
    let (mut request, started, _) = runtime_start_request(
        &manager,
        config,
        recognition_credential,
        Some(prepared_translation(
            selection.clone(),
            translation_credential.clone(),
        )?),
    )?;
    request.status_recorder = status_recorder.clone();
    let (snapshot_sender, snapshot_receiver) = mpsc::sync_channel(1);

    manager.start(app.handle().clone(), request, |snapshot| {
        snapshot_sender
            .send(snapshot)
            .map_err(|_| AppError::state("Runtime test dropped its generation-snapshot receiver."))
    })?;
    started
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Prepared Recognition Driver did not start."))?;
    let snapshot = snapshot_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Prepared generation snapshot was not installed."))?;

    assert_eq!(snapshot.selection.translation, Some(selection));
    assert_eq!(snapshot.credentials.len(), 2);
    let custom = snapshot
        .credentials
        .iter()
        .find(|credential| credential.id == CredentialId::CustomTranslation)
        .ok_or_else(|| AppError::state("Custom Translation credential was not disclosed."))?;
    assert_eq!(custom.storage, translation_credential.storage);
    assert_eq!(custom.display_suffix, translation_credential.display_suffix);
    assert_eq!(custom.revision, translation_credential.revision);
    assert!(snapshot.uploads_source_text);

    manager.stop(app.handle(), &status_recorder)
}

#[test]
fn active_translation_without_a_prepared_owner_keeps_the_module_unavailable_gate() -> AppResult<()>
{
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();
    let mut config = AppConfig {
        translation: Some(official_translation()),
        ..AppConfig::default()
    };
    config.publication.content = ContentSelection::TranslationOnly;
    let (request, _, _) = runtime_start_request(
        &manager,
        config,
        RuntimeGenerationCredentialSnapshot {
            id: CredentialId::OpenAi,
            storage: CredentialStorage::Environment,
            display_suffix: None,
            revision: 1,
        },
        None,
    )?;

    let error = manager
        .start(app.handle().clone(), request, |_| Ok(()))
        .err()
        .ok_or_else(|| AppError::state("Active Translation started without a prepared owner."))?;

    assert_eq!(error.code(), "config.invalid");
    assert!(error.to_string().contains("translation.module_unavailable"));
    assert!(
        manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?
            .is_none()
    );
    Ok(())
}

#[test]
fn runtime_rejects_translation_prepared_for_a_different_selection() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();
    let mut selected = official_translation();
    selected.target = TranslationTarget::English;
    let mut config = AppConfig {
        translation: Some(selected),
        ..AppConfig::default()
    };
    config.publication.content = ContentSelection::Bilingual;
    let credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::Environment,
        display_suffix: Some("same".to_string()),
        revision: 4,
    };
    let (request, _, _) = runtime_start_request(
        &manager,
        config,
        credential.clone(),
        Some(prepared_translation(official_translation(), credential)?),
    )?;

    let error = manager
        .start(app.handle().clone(), request, |_| Ok(()))
        .err()
        .ok_or_else(|| AppError::state("Runtime accepted a mismatched Translation selection."))?;

    assert_eq!(error.code(), "runtime.state_failed");
    assert!(error.to_string().contains("does not match"));
    assert!(
        manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?
            .is_none()
    );
    Ok(())
}

#[test]
fn official_translation_rejects_conflicting_shared_credential_metadata() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();
    let mut config = AppConfig {
        translation: Some(official_translation()),
        ..AppConfig::default()
    };
    config.publication.content = ContentSelection::TranslationOnly;
    let recognition_credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::Environment,
        display_suffix: Some("old".to_string()),
        revision: 4,
    };
    let translation_credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::SystemCredentialStore,
        display_suffix: Some("new".to_string()),
        revision: 5,
    };
    let (request, _, _) = runtime_start_request(
        &manager,
        config,
        recognition_credential,
        Some(prepared_translation(
            official_translation(),
            translation_credential,
        )?),
    )?;

    let error = manager
        .start(app.handle().clone(), request, |_| Ok(()))
        .err()
        .ok_or_else(|| AppError::state("Runtime accepted conflicting OpenAI metadata."))?;

    assert_eq!(error.code(), "runtime.state_failed");
    assert!(error.to_string().contains("disagree"));
    assert!(
        manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?
            .is_none()
    );
    Ok(())
}

#[test]
fn runtime_starts_the_prepared_module_with_its_bound_generation_metadata() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let manager = RuntimeManager::default();
    let mut config = AppConfig::default();
    config.osc.enabled = false;
    let credential = RuntimeGenerationCredentialSnapshot {
        id: CredentialId::OpenAi,
        storage: CredentialStorage::Environment,
        display_suffix: Some("bound".to_string()),
        revision: 3,
    };
    let (driver_started, started) = mpsc::sync_channel(1);
    let completed_runs = Arc::new(AtomicUsize::new(0));
    let recognition_module = RecognitionModule::with_audio_budget(
        Duration::from_millis(100),
        1,
        WaitForRuntimeStopDriver {
            started: driver_started,
            completed_runs: Arc::clone(&completed_runs),
        },
    )?;
    let (snapshot_sender, snapshot_receiver) = mpsc::sync_channel(1);
    let request = RuntimeStartRequest {
        config,
        chatbox_pacer: ChatboxPacer::default(),
        caption_aggregate: CaptionAggregateStore::default(),
        chatbox_host_resolver: HostResolver::default(),
        prepared_recognition: PreparedRecognition::cloud(recognition_module, credential)?,
        prepared_translation: None,
        generation_id: 1,
        config_revision: 2,
        status_recorder: status_recorder.clone(),
        expected_stop_epoch: manager.stop_epoch(),
    };

    assert_eq!(
        manager.start(app.handle().clone(), request, |snapshot| {
            snapshot_sender.send(snapshot).map_err(|_| {
                AppError::state("Runtime test dropped its generation-snapshot receiver.")
            })
        })?,
        RuntimeStartOutcome::Started
    );
    started
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Prepared Recognition Driver did not start."))?;
    let snapshot = snapshot_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::state("Prepared generation snapshot was not installed."))?;
    let snapshot_credential = snapshot
        .credentials
        .first()
        .ok_or_else(|| AppError::state("Prepared generation omitted its credential metadata."))?;
    assert_eq!(snapshot_credential.revision, 3);
    assert!(snapshot.uploads_microphone_audio);
    assert!(!snapshot.uploads_source_text);

    manager.stop(app.handle(), &status_recorder)?;
    assert_eq!(completed_runs.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn runtime_manager_closes_the_generation_before_joining_the_worker() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let generation = RuntimeGeneration::active();
    let manager = Arc::new(RuntimeManager::default());
    let (worker_ready_sender, worker_ready_receiver) = std::sync::mpsc::channel();
    let (release_worker_sender, release_worker_receiver) = std::sync::mpsc::channel();
    let join_handle = thread::spawn(move || {
        let _ = worker_ready_sender.send(());
        let _ = release_worker_receiver.recv();
    });

    {
        let mut handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        *handle = Some(RuntimeHandle {
            generation: generation.clone(),
            publisher: None,
            join_handle,
        });
    }
    worker_ready_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Runtime test worker did not start."))?;

    let stop_manager = Arc::clone(&manager);
    let stop_app = app.handle().clone();
    let stop_status_recorder = status_recorder.clone();
    let (stop_started_sender, stop_started_receiver) = std::sync::mpsc::channel();
    let stop = thread::spawn(move || {
        let _ = stop_started_sender.send(());
        stop_manager.stop(&stop_app, &stop_status_recorder)
    });
    stop_started_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Runtime stop test thread did not start."))?;

    let deadline = Instant::now() + Duration::from_secs(1);
    let stopping_before_join = loop {
        if generation.is_hard_stop_requested()
            && !generation.commit_if_active(|| {})?
            && control.snapshot()?.runtime_status.status == RuntimeStatus::Stopping
        {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };

    release_worker_sender
        .send(())
        .map_err(|_| AppError::runtime("Could not release the runtime test worker."))?;
    stop.join()
        .map_err(|_| AppError::runtime("Runtime stop test thread panicked."))??;
    assert!(stopping_before_join);
    assert_eq!(
        control.snapshot()?.runtime_status.status,
        RuntimeStatus::Stopped
    );
    Ok(())
}

#[test]
fn stop_records_error_when_the_runtime_thread_panicked() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let manager = RuntimeManager::default();
    let join_handle = thread::spawn(|| {
        std::panic::resume_unwind(Box::new("panic runtime worker for Stop coverage"));
    });
    {
        let mut handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        *handle = Some(RuntimeHandle {
            generation: RuntimeGeneration::active(),
            publisher: None,
            join_handle,
        });
    }

    let error = manager
        .stop(app.handle(), &status_recorder)
        .err()
        .ok_or_else(|| AppError::state("Stop ignored a panicked runtime thread."))?;

    assert_eq!(error.code(), "runtime.failed");
    assert_eq!(
        control.snapshot()?.runtime_status.status,
        RuntimeStatus::Error
    );
    Ok(())
}

#[test]
fn finished_error_handle_is_reaped_before_a_restart_availability_check() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();
    let (finished_sender, finished_receiver) = std::sync::mpsc::channel();
    let join_handle = thread::spawn(move || {
        let _ = finished_sender.send(());
    });
    finished_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Finished runtime test thread did not exit."))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while !join_handle.is_finished() {
        if Instant::now() >= deadline {
            return Err(AppError::runtime(
                "Finished runtime test thread did not become joinable.",
            ));
        }
        thread::yield_now();
    }
    {
        let mut handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        *handle = Some(RuntimeHandle {
            generation: RuntimeGeneration::active(),
            publisher: None,
            join_handle,
        });
    }

    manager.prepare_for_start(app.handle())?;
    let handle = manager
        .handle
        .lock()
        .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
    assert!(handle.is_none());
    Ok(())
}

#[test]
fn stop_invalidates_an_uncommitted_start_epoch() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let manager = RuntimeManager::default();
    let expected_stop_epoch = manager.stop_epoch();

    assert!(manager.stop_epoch_unchanged(expected_stop_epoch));
    manager.stop(app.handle(), &status_recorder)?;
    assert!(!manager.stop_epoch_unchanged(expected_stop_epoch));
    let snapshot = control.snapshot()?;
    assert_eq!(snapshot.runtime_status.status, RuntimeStatus::Stopped);
    assert_eq!(
        snapshot.runtime_status.message.as_deref(),
        Some("Runtime is already stopped")
    );
    Ok(())
}

#[test]
fn stop_supersedes_a_start_blocked_in_osc_hostname_resolution() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let (diagnostic_sender, diagnostic_receiver) = mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let manager = Arc::new(RuntimeManager::default());
    let (lookup_started_sender, lookup_started_receiver) = mpsc::sync_channel(1);
    let (lookup_release_sender, lookup_release_receiver) = mpsc::sync_channel(1);
    let lookup_release_receiver = Arc::new(Mutex::new(lookup_release_receiver));
    let worker_release = Arc::clone(&lookup_release_receiver);
    let resolver = HostResolver::with_lookup(move |_, port| {
        let _ = lookup_started_sender.send(());
        worker_release
            .lock()
            .map_err(|_| io::Error::other("Test resolver release lock was poisoned."))?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::other("Test resolver was not released before its timeout."))?;
        Ok(vec![std::net::SocketAddr::from(([127, 0, 0, 1], port))])
    });
    let mut config = AppConfig::default();
    config.osc.host = "blocked.test".to_string();
    let recognition_module = test_recognition_module(&config)?;
    let expected_stop_epoch = manager.stop_epoch();
    let request = RuntimeStartRequest {
        config,
        chatbox_pacer: ChatboxPacer::default(),
        caption_aggregate: CaptionAggregateStore::default(),
        chatbox_host_resolver: resolver,
        prepared_recognition: PreparedRecognition::cloud(
            recognition_module,
            RuntimeGenerationCredentialSnapshot {
                id: CredentialId::OpenAi,
                storage: CredentialStorage::Environment,
                display_suffix: None,
                revision: 1,
            },
        )?,
        prepared_translation: None,
        generation_id: 1,
        config_revision: 1,
        status_recorder: status_recorder.clone(),
        expected_stop_epoch,
    };
    let start_manager = Arc::clone(&manager);
    let start_app = app.handle().clone();
    let start = thread::spawn(move || start_manager.start(start_app, request, |_| Ok(())));
    lookup_started_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| AppError::state("OSC hostname resolution did not start."))?;

    let stop_manager = Arc::clone(&manager);
    let stop_app = app.handle().clone();
    let stop_status_recorder = status_recorder.clone();
    let (stop_result_sender, stop_result_receiver) = mpsc::sync_channel(1);
    let stop = thread::spawn(move || {
        let _ = stop_result_sender.send(stop_manager.stop(&stop_app, &stop_status_recorder));
    });
    let stop_result = stop_result_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| AppError::state("Stop waited for the blocked OS hostname lookup."))?;
    stop_result?;
    let start_outcome = start
        .join()
        .map_err(|_| AppError::state("Blocked runtime Start thread panicked."))??;

    assert_eq!(start_outcome, RuntimeStartOutcome::SupersededByStop);
    assert!(
        manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?
            .is_none()
    );
    for payload in diagnostic_receiver.try_iter() {
        let diagnostic = serde_json::from_str::<serde_json::Value>(&payload).map_err(|error| {
            AppError::state(format!("Runtime diagnostic was not valid JSON: {error}"))
        })?;
        assert_ne!(diagnostic["code"], "osc.send_failed");
    }

    lookup_release_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the blocked hostname lookup."))?;
    stop.join()
        .map_err(|_| AppError::state("Runtime Stop thread panicked."))?;
    Ok(())
}

#[test]
fn stop_cancels_an_installed_runtime_hostname_wait_before_joining() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let manager = Arc::new(RuntimeManager::default());
    let generation = RuntimeGeneration::active();
    let worker_generation = generation.clone();
    let (lookup_started_sender, lookup_started_receiver) = mpsc::sync_channel(1);
    let (lookup_release_sender, lookup_release_receiver) = mpsc::sync_channel(1);
    let lookup_release_receiver = Arc::new(Mutex::new(lookup_release_receiver));
    let worker_release = Arc::clone(&lookup_release_receiver);
    let resolver = HostResolver::with_lookup(move |_, port| {
        let _ = lookup_started_sender.send(());
        worker_release
            .lock()
            .map_err(|_| io::Error::other("Test resolver release lock was poisoned."))?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::other("Test resolver was not released before its timeout."))?;
        Ok(vec![std::net::SocketAddr::from(([127, 0, 0, 1], port))])
    });
    let (resolution_sender, resolution_receiver) = mpsc::sync_channel(1);
    let join_handle = thread::spawn(move || {
        let result = resolver.resolve_until(
            "blocked-openai.test",
            443,
            Instant::now() + Duration::from_secs(5),
            &|| worker_generation.is_work_cancelled(),
        );
        let _ = resolution_sender.send(result);
    });
    {
        let mut handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        *handle = Some(RuntimeHandle {
            generation,
            publisher: None,
            join_handle,
        });
    }
    lookup_started_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| AppError::state("Installed runtime hostname lookup did not start."))?;

    let stop_manager = Arc::clone(&manager);
    let stop_app = app.handle().clone();
    let stop_status_recorder = status_recorder.clone();
    let (stop_result_sender, stop_result_receiver) = mpsc::sync_channel(1);
    let stop = thread::spawn(move || {
        let _ = stop_result_sender.send(stop_manager.stop(&stop_app, &stop_status_recorder));
    });
    let stop_result = stop_result_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| {
            AppError::state("Stop waited for the installed runtime's OS hostname lookup.")
        })?;
    stop_result?;
    let resolution = resolution_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| {
            AppError::state("Runtime hostname wait did not observe generation cancellation.")
        })?;

    assert_eq!(resolution.err(), Some(HostResolutionError::Cancelled));
    lookup_release_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the installed runtime hostname lookup."))?;
    stop.join()
        .map_err(|_| AppError::state("Installed runtime Stop thread panicked."))?;
    Ok(())
}

#[test]
fn runtime_start_fails_closed_when_its_derived_plan_is_incompatible() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();
    let mut config = AppConfig::default();
    config.publication.mode = crate::config::PublicationMode::Live;
    let recognition_module = test_recognition_module(&config)?;
    let request = RuntimeStartRequest {
        config,
        chatbox_pacer: ChatboxPacer::default(),
        caption_aggregate: CaptionAggregateStore::default(),
        chatbox_host_resolver: HostResolver::default(),
        prepared_recognition: PreparedRecognition::cloud(
            recognition_module,
            RuntimeGenerationCredentialSnapshot {
                id: CredentialId::OpenAi,
                storage: CredentialStorage::Environment,
                display_suffix: None,
                revision: 1,
            },
        )?,
        prepared_translation: None,
        generation_id: 1,
        config_revision: 1,
        status_recorder: RuntimeControlStore::default().status_recorder(),
        expected_stop_epoch: manager.stop_epoch(),
    };

    let error = manager
        .start(app.handle().clone(), request, |_| Ok(()))
        .err()
        .ok_or_else(|| {
            AppError::state("Incompatible runtime configuration unexpectedly started.")
        })?;
    assert_eq!(error.code(), "config.invalid");
    assert!(error.to_string().contains("publication.mode_unsupported"));
    assert!(
        manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?
            .is_none()
    );
    Ok(())
}

#[test]
fn microphone_probe_lease_excludes_runtime_start_until_released() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();

    let probe = manager.begin_audio_probe(app.handle())?;
    let start_error = manager
        .prepare_for_start(app.handle())
        .err()
        .ok_or_else(|| AppError::state("Runtime start ignored the active microphone probe."))?;
    assert_eq!(start_error.code(), "runtime.failed");

    drop(probe);
    manager.prepare_for_start(app.handle())?;
    Ok(())
}
