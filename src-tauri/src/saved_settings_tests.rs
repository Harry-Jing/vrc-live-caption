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
fn save_writes_current_settings_through_the_temporary_path() -> AppResult<()> {
    let directory = TestSettingsDirectory::new("save")?;
    let path = directory.config_path();
    let mut config = AppConfig::default();
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

    assert_eq!(value.get("schemaVersion"), Some(&serde_json::json!(3)));
    assert_eq!(
        value.pointer("/publication/mode"),
        Some(&serde_json::json!("completed"))
    );
    assert!(value.pointer("/osc/minIntervalMs").is_none());

    Ok(())
}

#[test]
fn current_config_round_trips_without_compatibility_defaults() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.audio.input_device_id = Some("saved-device".to_string());
    config.stt.languages = vec!["zh".to_string(), "en".to_string()];
    config.stt.model = crate::config::OpenAiTranscriptionModel::GptLiveTranscribe;
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
fn current_live_publication_round_trips() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.stt.model = crate::config::OpenAiTranscriptionModel::GptLiveTranscribe;
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
fn parse_valid_config_rejects_removed_singular_language() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    let stt = value
        .get_mut("stt")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| AppError::state("Test config is missing stt."))?;
    stt.remove("languages");
    stt.insert("language".to_string(), serde_json::json!("en"));

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_rejects_removed_mock_provider_and_arbitrary_model() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["stt"]["provider"] = serde_json::json!("mock");
    value["stt"]["model"] = serde_json::json!("saved-model");

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_ignores_only_the_removed_osc_interval() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["osc"]["host"] = serde_json::json!("192.0.2.25");
    value["osc"]["minIntervalMs"] = serde_json::json!(750);

    let config = parse_valid_config(&value.to_string())?;
    assert_eq!(config.osc.host, "192.0.2.25");
    assert!(
        serde_json::to_value(config)
            .map_err(|error| AppError::config(format!("Failed to serialize config: {error}")))?
            .pointer("/osc/minIntervalMs")
            .is_none()
    );
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
fn parse_valid_config_rejects_old_schema_version() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["schemaVersion"] = serde_json::json!(2);

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}
