use super::test_support::receive_json_event;
use super::*;
use crate::chatbox::{LivePublisherDiagnostic, PublisherDiagnostic};
use crate::config::SttProvider;
use crate::host_resolver::{HostResolutionError, HostResolver};
use crate::secrets::ProviderSecretStorage;
use secrecy::SecretString;
use std::io;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tauri::Listener;

#[test]
fn runtime_manager_closes_the_generation_before_joining_the_worker() -> AppResult<()> {
    let app = tauri::test::mock_app();
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
    let (stop_started_sender, stop_started_receiver) = std::sync::mpsc::channel();
    let stop = thread::spawn(move || {
        let _ = stop_started_sender.send(());
        stop_manager.stop(&stop_app)
    });
    stop_started_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Runtime stop test thread did not start."))?;

    let deadline = Instant::now() + Duration::from_secs(1);
    let generation_closed_before_join = loop {
        if generation.is_hard_stop_requested() && !generation.commit_if_active(|| {})? {
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
    assert!(generation_closed_before_join);
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
    let manager = RuntimeManager::default();
    let expected_stop_epoch = manager.stop_epoch();

    assert!(manager.stop_epoch_unchanged(expected_stop_epoch));
    manager.stop(app.handle())?;
    assert!(!manager.stop_epoch_unchanged(expected_stop_epoch));
    Ok(())
}

#[test]
fn stop_supersedes_a_start_blocked_in_osc_hostname_resolution() -> AppResult<()> {
    let app = tauri::test::mock_app();
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
    let expected_stop_epoch = manager.stop_epoch();
    let request = RuntimeStartRequest {
        config,
        chatbox_pacer: ChatboxPacer::default(),
        caption_session: CaptionSessionStore::default(),
        host_resolver: resolver,
        generation_id: 1,
        config_revision: 1,
        openai_api_key: SecretString::from("test-key".to_string()),
        credential: RuntimeCredentialSnapshot {
            provider: SttProvider::OpenAi,
            storage: ProviderSecretStorage::Environment,
            display_suffix: None,
            revision: 1,
        },
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
    let (stop_result_sender, stop_result_receiver) = mpsc::sync_channel(1);
    let stop = thread::spawn(move || {
        let _ = stop_result_sender.send(stop_manager.stop(&stop_app));
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
    let (stop_result_sender, stop_result_receiver) = mpsc::sync_channel(1);
    let stop = thread::spawn(move || {
        let _ = stop_result_sender.send(stop_manager.stop(&stop_app));
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
    let request = RuntimeStartRequest {
        config,
        chatbox_pacer: ChatboxPacer::default(),
        caption_session: CaptionSessionStore::default(),
        host_resolver: HostResolver::default(),
        generation_id: 1,
        config_revision: 1,
        openai_api_key: SecretString::from("test-key".to_string()),
        credential: RuntimeCredentialSnapshot {
            provider: SttProvider::OpenAi,
            storage: ProviderSecretStorage::Environment,
            display_suffix: None,
            revision: 1,
        },
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

#[test]
fn publisher_diagnostics_keep_stable_osc_wire_codes() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let diagnostics = vec![
        (
            PublisherDiagnostic::UnitPublished {
                unit_id: "published".to_string(),
                page_count: 2,
                byte_count: 42,
                target: "127.0.0.1:9000".to_string(),
            },
            "osc.completed_unit_sent",
            "info",
        ),
        (
            PublisherDiagnostic::UnitDroppedOverload {
                unit_id: "dropped".to_string(),
                page_count: 2,
            },
            "osc.completed_unit_dropped_overload",
            "warning",
        ),
        (
            PublisherDiagnostic::UnitRejectedOverload {
                unit_id: "rejected".to_string(),
                page_count: 33,
            },
            "osc.completed_unit_rejected_overload",
            "warning",
        ),
        (
            PublisherDiagnostic::UnitExpired {
                unit_id: "expired".to_string(),
                page_count: 2,
            },
            "osc.completed_unit_expired",
            "warning",
        ),
        (
            PublisherDiagnostic::LayoutFailed {
                unit_id: "layout".to_string(),
                reason: "test layout failure".to_string(),
            },
            "osc.completed_layout_failed",
            "warning",
        ),
        (
            PublisherDiagnostic::UnitSendFailed {
                unit_id: "send".to_string(),
                page_index: 2,
                page_count: 3,
                pages_sent: 1,
                error: AppError::osc_send("test", "send failure".to_string()),
            },
            "osc.send_failed",
            "error",
        ),
        (
            PublisherDiagnostic::PagesDiscardedOnClose {
                reason: PublisherCloseReason::Stop,
                unit_count: 2,
                page_count: 3,
                started_unit_count: 1,
            },
            "osc.completed_pages_discarded_on_stop",
            "info",
        ),
        (
            PublisherDiagnostic::PagesDiscardedOnClose {
                reason: PublisherCloseReason::RuntimeError,
                unit_count: 2,
                page_count: 3,
                started_unit_count: 1,
            },
            "osc.completed_pages_discarded_on_error",
            "info",
        ),
        (
            PublisherDiagnostic::TypingFailed {
                is_typing: false,
                error: AppError::osc_send("test", "typing failure".to_string()),
            },
            "osc.send_failed",
            "error",
        ),
        (
            PublisherDiagnostic::WorkerFailed {
                reason: "worker failure".to_string(),
            },
            "osc.completed_publisher_failed",
            "error",
        ),
    ];

    for (diagnostic, expected_code, expected_severity) in diagnostics {
        emit_diagnostic(app.handle(), completed_publisher_diagnostic(diagnostic));
        let event = receive_json_event(&diagnostic_receiver, "Publisher diagnostic")?;
        assert_eq!(event["category"], "osc");
        assert_eq!(event["code"], expected_code);
        assert_eq!(event["severity"], expected_severity);
        if expected_code == "osc.completed_unit_rejected_overload" {
            assert!(
                event["detail"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("No partial pages were queued")
            );
        }
        if expected_code == "osc.completed_pages_discarded_on_stop" {
            assert!(
                event["detail"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("Discarded 3 unsent page(s)")
            );
        }
    }
    Ok(())
}

#[test]
fn live_publisher_diagnostics_keep_stable_osc_wire_codes() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let diagnostics = vec![
        (
            LivePublisherDiagnostic::ViewPublished {
                stream_id: "recognition-1-1".to_string(),
                unit_id: Some("unit-1".to_string()),
                revision: 2,
                byte_count: 12,
                target: "127.0.0.1:9000".to_string(),
            },
            "osc.live_view_sent",
            "info",
        ),
        (
            LivePublisherDiagnostic::ViewSendFailed {
                stream_id: "recognition-1-1".to_string(),
                unit_id: None,
                revision: 3,
                error: AppError::osc_send("test", "send failure".to_string()),
            },
            "osc.live_view_send_failed",
            "error",
        ),
        (
            LivePublisherDiagnostic::LayoutFailed {
                stream_id: "recognition-1-1".to_string(),
                unit_id: Some("unit-2".to_string()),
                revision: 4,
                reason: "layout failure".to_string(),
            },
            "osc.live_layout_failed",
            "warning",
        ),
        (
            LivePublisherDiagnostic::DraftDiscardedOnClose {
                reason: PublisherCloseReason::Stop,
            },
            "osc.live_draft_discarded_on_stop",
            "info",
        ),
        (
            LivePublisherDiagnostic::DraftDiscardedOnClose {
                reason: PublisherCloseReason::RuntimeError,
            },
            "osc.live_draft_discarded_on_error",
            "info",
        ),
        (
            LivePublisherDiagnostic::TypingFailed {
                error: AppError::osc_send("test", "typing failure".to_string()),
            },
            "osc.live_typing_failed",
            "error",
        ),
        (
            LivePublisherDiagnostic::WorkerFailed {
                reason: "worker failure".to_string(),
            },
            "osc.live_publisher_failed",
            "error",
        ),
    ];

    for (diagnostic, expected_code, expected_severity) in diagnostics {
        emit_diagnostic(app.handle(), live_publisher_diagnostic(diagnostic));
        let event = receive_json_event(&diagnostic_receiver, "Live publisher diagnostic")?;
        assert_eq!(event["category"], "osc");
        assert_eq!(event["code"], expected_code);
        assert_eq!(event["severity"], expected_severity);
    }
    Ok(())
}
