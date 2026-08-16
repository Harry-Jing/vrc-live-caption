use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestSettingsDirectory(PathBuf);

impl TestSettingsDirectory {
    fn new(label: &str) -> AppResult<Self> {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vrc-live-caption-saved-settings-{}-{sequence}-{label}",
            std::process::id()
        ));
        fs::create_dir_all(&path).map_err(|error| {
            AppError::config_io(format!(
                "Failed to create saved-settings test directory at {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self(path))
    }

    fn config_path(&self) -> PathBuf {
        self.0.join(CONFIG_FILE_NAME)
    }
}

impl Drop for TestSettingsDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_saved_settings_load_editable_defaults_without_review() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("missing")?;

    match load_from_path(directory.config_path())? {
        SavedSettingsLoad::Ready(config) => assert_eq!(config, AppConfig::default()),
        SavedSettingsLoad::DefaultsRequireReview { .. } => {
            return Err(AppError::state(
                "Missing saved settings unexpectedly required review.",
            ));
        }
    }

    Ok(())
}

#[test]
fn supported_v1_settings_migrate_to_v2_in_memory_without_rewriting_the_file() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("v1-migration")?;
    let path = directory.config_path();
    let contents = serde_json::json!({
        "schemaVersion": 1,
        "audio": { "inputDeviceId": "saved-device" },
        "recognition": {
            "path": "openai/gpt-live-transcribe",
            "expectedLanguages": ["zh", "en"]
        },
        "osc": { "host": "192.0.2.25", "port": 9001, "enabled": false },
        "publication": { "mode": "live" },
        "ui": { "showOngoingPreview": false }
    })
    .to_string();
    fs::write(&path, &contents).map_err(|error| {
        AppError::config_io(format!(
            "Failed to write V1 migration fixture at {}: {error}",
            path.display()
        ))
    })?;

    let migrated = match load_from_path(path.clone())? {
        SavedSettingsLoad::Ready(config) => config,
        SavedSettingsLoad::DefaultsRequireReview { .. } => {
            return Err(AppError::state(
                "Supported V1 settings unexpectedly required review.",
            ));
        }
    };
    let value = serde_json::to_value(migrated).map_err(|error| {
        AppError::state(format!("Failed to serialize migrated config: {error}"))
    })?;

    assert_eq!(value["schemaVersion"], serde_json::json!(2));
    assert_eq!(value["audio"]["inputDeviceId"], "saved-device");
    assert_eq!(value["recognition"]["path"], "openai/gpt-live-transcribe");
    assert_eq!(
        value["recognition"]["expectedLanguages"],
        serde_json::json!(["zh", "en"])
    );
    assert_eq!(value["osc"]["host"], "192.0.2.25");
    assert_eq!(value["osc"]["port"], 9001);
    assert_eq!(value["osc"]["enabled"], false);
    assert_eq!(value["publication"]["mode"], "live");
    assert_eq!(value["publication"]["content"], "sourceOnly");
    assert_eq!(value["translation"], serde_json::Value::Null);
    assert_eq!(value["ui"]["showOngoingPreview"], false);
    assert_eq!(
        fs::read_to_string(path).map_err(|error| {
            AppError::config_io(format!("Failed to reread V1 migration fixture: {error}"))
        })?,
        contents
    );

    Ok(())
}

#[test]
fn invalid_saved_settings_load_defaults_with_review_context() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("invalid")?;
    let path = directory.config_path();
    fs::write(&path, "{ not json").map_err(|error| {
        AppError::config_io(format!(
            "Failed to write invalid saved settings at {}: {error}",
            path.display()
        ))
    })?;

    match load_from_path(path.clone())? {
        SavedSettingsLoad::Ready(_) => {
            return Err(AppError::state(
                "Invalid saved settings unexpectedly loaded without review.",
            ));
        }
        SavedSettingsLoad::DefaultsRequireReview {
            config,
            path: reported_path,
            error,
        } => {
            assert_eq!(config, AppConfig::default());
            assert_eq!(reported_path, path);
            assert_eq!(error.code(), "config.io_failed");
        }
    }

    Ok(())
}

#[test]
fn current_v2_settings_save_and_load_through_the_temporary_path() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("save")?;
    let path = directory.config_path();
    let mut config = AppConfig::default();
    assert_eq!(config.schema_version, 2);
    config.audio.input_device_id = Some("saved-device".to_string());

    save_to_path(&path, &config)?;

    assert!(!path.with_extension("json.tmp").exists());
    match load_from_path(path)? {
        SavedSettingsLoad::Ready(saved) => assert_eq!(saved, config),
        SavedSettingsLoad::DefaultsRequireReview { .. } => {
            return Err(AppError::state(
                "Freshly saved settings unexpectedly required review.",
            ));
        }
    }

    Ok(())
}

#[test]
fn v2_custom_url_loads_without_rewrite_or_compatibility_defaults() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("v2-custom-url-compatibility")?;
    let path = directory.config_path();
    let contents = serde_json::json!({
        "schemaVersion": 2,
        "audio": { "inputDeviceId": "saved-translation-device" },
        "recognition": {
            "path": "openai/gpt-live-transcribe",
            "expectedLanguages": ["zh", "en"]
        },
        "translation": {
            "path": "openai/responses-completed-text",
            "target": "zh-Hans",
            "endpoint": {
                "kind": "custom",
                "apiBaseUrl": " https://translation.example.test/api/v1"
            }
        },
        "osc": { "host": "192.0.2.25", "port": 9001, "enabled": false },
        "publication": { "mode": "completed", "content": "bilingual" },
        "ui": { "showOngoingPreview": false }
    })
    .to_string();
    fs::write(&path, &contents).map_err(|error| {
        AppError::config_io(format!(
            "Failed to write V2 compatibility fixture at {}: {error}",
            path.display()
        ))
    })?;

    let loaded = match load_from_path(path.clone())? {
        SavedSettingsLoad::Ready(config) => config,
        SavedSettingsLoad::DefaultsRequireReview { .. } => {
            return Err(AppError::state(
                "Supported V2 Custom URL unexpectedly loaded defaults.",
            ));
        }
    };

    assert_eq!(
        loaded.audio.input_device_id.as_deref(),
        Some("saved-translation-device")
    );
    assert_eq!(loaded.publication.content, ContentSelection::Bilingual);
    assert!(!loaded.osc.enabled);
    let api_base_url = match loaded.translation {
        Some(crate::config::TranslationConfig {
            endpoint: crate::config::TranslationEndpoint::Custom { api_base_url },
            ..
        }) => api_base_url,
        _ => {
            return Err(AppError::state(
                "Supported V2 Custom Translation selection was not retained.",
            ));
        }
    };
    assert_eq!(
        api_base_url.as_url().as_str(),
        "https://translation.example.test/api/v1"
    );
    assert_eq!(
        fs::read_to_string(path).map_err(|error| {
            AppError::config_io(format!(
                "Failed to reread V2 compatibility fixture: {error}"
            ))
        })?,
        contents
    );

    Ok(())
}

#[test]
fn custom_translation_v2_round_trips_with_the_exact_persisted_shape() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("custom-translation-round-trip")?;
    let path = directory.config_path();
    let mut config = AppConfig::default();
    config.audio.input_device_id = Some("translation-device".to_string());
    config.recognition.path = crate::config::RecognitionPath::OpenAiGptLiveTranscribe;
    config.recognition.expected_languages = vec!["zh".to_string(), "en".to_string()];
    config.translation = Some(crate::config::TranslationConfig {
        path: crate::config::TranslationPath::OpenAiResponsesCompletedText,
        target: crate::config::TranslationTarget::SimplifiedChinese,
        endpoint: crate::config::TranslationEndpoint::Custom {
            api_base_url: crate::config::ApiBaseUrl::parse(
                "https://translation.example.test/api/v1",
            )
            .map_err(AppError::config)?,
        },
    });
    config.osc.host = "192.0.2.25".to_string();
    config.osc.port = 9001;
    config.osc.enabled = false;
    config.publication.content = ContentSelection::Bilingual;
    config.ui.show_ongoing_preview = false;

    save_to_path(&path, &config)?;

    let persisted_contents = fs::read_to_string(&path).map_err(|error| {
        AppError::config_io(format!(
            "Failed to read Custom Translation settings at {}: {error}",
            path.display()
        ))
    })?;
    let persisted =
        serde_json::from_str::<serde_json::Value>(&persisted_contents).map_err(|error| {
            AppError::state(format!("Failed to parse persisted test config: {error}"))
        })?;
    assert_eq!(
        persisted,
        serde_json::json!({
            "schemaVersion": 2,
            "audio": { "inputDeviceId": "translation-device" },
            "recognition": {
                "path": "openai/gpt-live-transcribe",
                "expectedLanguages": ["zh", "en"]
            },
            "translation": {
                "path": "openai/responses-completed-text",
                "target": "zh-Hans",
                "endpoint": {
                    "kind": "custom",
                    "apiBaseUrl": "https://translation.example.test/api/v1"
                }
            },
            "osc": { "host": "192.0.2.25", "port": 9001, "enabled": false },
            "publication": { "mode": "completed", "content": "bilingual" },
            "ui": { "showOngoingPreview": false }
        })
    );

    match load_from_path(path)? {
        SavedSettingsLoad::Ready(saved) => assert_eq!(saved, config),
        SavedSettingsLoad::DefaultsRequireReview { .. } => {
            return Err(AppError::state(
                "Freshly saved Custom Translation settings unexpectedly required review.",
            ));
        }
    }

    Ok(())
}

#[test]
fn save_replaces_existing_settings_without_leaving_the_temporary_file() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("replace")?;
    let path = directory.config_path();
    let mut original = AppConfig::default();
    original.audio.input_device_id = Some("old-device".to_string());
    save_to_path(&path, &original)?;

    let mut replacement = original;
    replacement.audio.input_device_id = Some("new-device".to_string());
    replacement.osc.enabled = false;
    save_to_path(&path, &replacement)?;

    assert!(!path.with_extension("json.tmp").exists());
    match load_from_path(path)? {
        SavedSettingsLoad::Ready(saved) => assert_eq!(saved, replacement),
        SavedSettingsLoad::DefaultsRequireReview { .. } => {
            return Err(AppError::state(
                "Replacement settings unexpectedly required review.",
            ));
        }
    }

    Ok(())
}

#[test]
fn failed_temporary_write_preserves_existing_settings() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("write-failure")?;
    let path = directory.config_path();
    let mut original = AppConfig::default();
    original.audio.input_device_id = Some("preserved-device".to_string());
    save_to_path(&path, &original)?;

    let temp_path = path.with_extension("json.tmp");
    fs::create_dir(&temp_path).map_err(|error| {
        AppError::config_io(format!(
            "Failed to block the temporary settings path at {}: {error}",
            temp_path.display()
        ))
    })?;
    let mut replacement = original.clone();
    replacement.audio.input_device_id = Some("discarded-device".to_string());

    let error = save_to_path(&path, &replacement)
        .err()
        .ok_or_else(|| AppError::state("Blocked temporary settings path unexpectedly saved."))?;
    assert_eq!(error.code(), "config.io_failed");
    match load_from_path(path)? {
        SavedSettingsLoad::Ready(saved) => assert_eq!(saved, original),
        SavedSettingsLoad::DefaultsRequireReview { .. } => {
            return Err(AppError::state(
                "Failed replacement damaged the existing settings.",
            ));
        }
    }

    Ok(())
}

#[test]
fn default_config_serializes_schema_version() -> Result<(), serde_json::Error> {
    let value = serde_json::to_value(AppConfig::default())?;

    assert_eq!(value.get("schemaVersion"), Some(&serde_json::json!(2)));
    assert_eq!(
        value.pointer("/recognition/path"),
        Some(&serde_json::json!("openai/gpt-transcribe"))
    );
    assert!(value.get("stt").is_none());
    assert!(value.pointer("/ui/showPartial").is_none());
    assert_eq!(
        value.pointer("/publication/mode"),
        Some(&serde_json::json!("completed"))
    );
    assert_eq!(
        value.pointer("/publication/content"),
        Some(&serde_json::json!("sourceOnly"))
    );
    assert_eq!(value.get("translation"), Some(&serde_json::Value::Null));
    assert!(value.pointer("/osc/minIntervalMs").is_none());

    Ok(())
}

#[test]
fn current_config_round_trips_without_compatibility_defaults() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.audio.input_device_id = Some("saved-device".to_string());
    config.recognition.expected_languages = vec!["zh".to_string(), "en".to_string()];
    config.recognition.path = crate::config::RecognitionPath::OpenAiGptLiveTranscribe;
    config.osc.enabled = false;
    config.publication.mode = crate::config::PublicationMode::Live;
    let serialized = serde_json::to_string(&config).map_err(|error| {
        AppError::config_io(format!("Failed to serialize test config: {error}"))
    })?;
    let reparsed = parse_valid_config(&serialized)?;

    assert_eq!(reparsed, config);
    Ok(())
}

#[test]
fn current_v2_requires_an_explicit_translation_field() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::state(format!("Failed to build V2 test config: {error}")))?;
    value
        .as_object_mut()
        .ok_or_else(|| AppError::state("V2 test config was not an object."))?
        .remove("translation");

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn current_live_publication_round_trips() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.recognition.path = crate::config::RecognitionPath::OpenAiGptLiveTranscribe;
    config.publication.mode = crate::config::PublicationMode::Live;
    let serialized = serde_json::to_string(&config).map_err(|error| {
        AppError::config_io(format!("Failed to serialize test config: {error}"))
    })?;
    let reparsed = parse_valid_config(&serialized)?;

    assert_eq!(reparsed, config);
    assert_eq!(
        reparsed.publication.mode,
        crate::config::PublicationMode::Live
    );

    Ok(())
}

#[test]
fn parse_valid_config_rejects_malformed_json() {
    assert!(parse_valid_config("{ not json").is_err());
}

#[test]
fn parse_valid_config_rejects_removed_recognition_aliases() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    let recognition = value
        .get_mut("recognition")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| AppError::state("Test config is missing recognition."))?;
    recognition.remove("expectedLanguages");
    recognition.insert("languages".to_string(), serde_json::json!(["en"]));

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_rejects_arbitrary_recognition_path() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["recognition"]["path"] = serde_json::json!("mock/saved-model");

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_rejects_the_removed_osc_interval() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["osc"]["host"] = serde_json::json!("192.0.2.25");
    value["osc"]["minIntervalMs"] = serde_json::json!(750);

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_still_rejects_other_unknown_fields() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["osc"]["unknownSetting"] = serde_json::json!(true);

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_rejects_unsupported_schema_version() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["schemaVersion"] = serde_json::json!(3);

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn pre_baseline_v1_through_v4_require_review_without_rewriting_the_file() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("pre-baseline")?;
    let path = directory.config_path();
    let configs = [
        (
            "V1",
            serde_json::json!({
                "schemaVersion": 1,
                "audio": { "inputDeviceId": "v1-device" },
                "stt": {
                    "provider": "openai",
                    "language": "en",
                    "model": "gpt-4o-mini-transcribe"
                },
                "osc": {
                    "host": "192.0.2.1",
                    "port": 9001,
                    "enabled": true,
                    "minIntervalMs": 1200
                },
                "ui": { "showPartial": true }
            }),
        ),
        (
            "V2",
            serde_json::json!({
                "schemaVersion": 2,
                "audio": { "inputDeviceId": "v2-device" },
                "stt": {
                    "provider": "openai",
                    "language": "zh",
                    "model": "gpt-4o-mini-transcribe"
                },
                "osc": { "host": "192.0.2.2", "port": 9002, "enabled": false },
                "publication": { "mode": "completed" },
                "ui": { "showPartial": false }
            }),
        ),
        (
            "V3",
            serde_json::json!({
                "schemaVersion": 3,
                "audio": { "inputDeviceId": "v3-device" },
                "stt": {
                    "provider": "openai",
                    "languages": ["zh", "en"],
                    "model": "gpt-live-transcribe"
                },
                "osc": { "host": "192.0.2.3", "port": 9003, "enabled": false },
                "publication": { "mode": "live" },
                "ui": { "showPartial": false }
            }),
        ),
        (
            "V4",
            serde_json::json!({
                "schemaVersion": 4,
                "audio": { "inputDeviceId": "v4-device" },
                "recognition": {
                    "path": "openai/gpt-live-transcribe",
                    "expectedLanguages": ["zh", "en"]
                },
                "osc": { "host": "192.0.2.4", "port": 9004, "enabled": false },
                "publication": { "mode": "live" },
                "ui": { "showOngoingPreview": false }
            }),
        ),
    ];

    for (version, config) in configs {
        let contents = serde_json::to_string_pretty(&config).map_err(|error| {
            AppError::state(format!(
                "Failed to serialize pre-baseline {version}: {error}"
            ))
        })?;
        fs::write(&path, &contents).map_err(|error| {
            AppError::config_io(format!(
                "Failed to write pre-baseline {version} settings at {}: {error}",
                path.display()
            ))
        })?;

        match load_from_path(path.clone())? {
            SavedSettingsLoad::Ready(_) => {
                return Err(AppError::state(format!(
                    "Pre-baseline {version} settings unexpectedly loaded without review."
                )));
            }
            SavedSettingsLoad::DefaultsRequireReview {
                config,
                path: reported_path,
                error,
            } => {
                assert_eq!(config, AppConfig::default());
                assert_eq!(reported_path, path);
                assert_eq!(error.code(), "config.io_failed");
            }
        }
        let preserved = fs::read_to_string(&path).map_err(|error| {
            AppError::config_io(format!(
                "Failed to reread pre-baseline {version} settings at {}: {error}",
                path.display()
            ))
        })?;
        assert_eq!(preserved, contents);
    }

    Ok(())
}
