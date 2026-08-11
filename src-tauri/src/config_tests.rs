use super::*;

#[test]
fn default_config_selects_the_gpt_transcribe_recognition_path_with_language_hints() {
    let config = AppConfig::default();

    assert_eq!(config.schema_version, 1);
    assert_eq!(
        config.recognition.path,
        RecognitionPath::OpenAiGptTranscribe
    );
    assert_eq!(config.recognition.expected_languages, vec!["en"]);
    assert!(config.ui.show_ongoing_preview);
}

#[test]
fn config_accepts_only_the_two_exact_recognition_paths() -> AppResult<()> {
    for (path, expected) in [
        (
            "openai/gpt-transcribe",
            RecognitionPath::OpenAiGptTranscribe,
        ),
        (
            "openai/gpt-live-transcribe",
            RecognitionPath::OpenAiGptLiveTranscribe,
        ),
    ] {
        let payload = valid_config_json(path);
        let config: AppConfig = serde_json::from_value(payload).map_err(|error| {
            AppError::state(format!("Supported path failed to deserialize: {error}"))
        })?;

        assert_eq!(config.recognition.path, expected);
    }

    let legacy = valid_config_json("openai/gpt-4o-mini-transcribe");
    assert!(serde_json::from_value::<AppConfig>(legacy).is_err());
    Ok(())
}

#[test]
fn config_rejects_pre_baseline_recognition_fields_in_the_current_v1_schema() -> AppResult<()> {
    let mut legacy_language = valid_config_json("openai/gpt-transcribe");
    let recognition = legacy_language["recognition"]
        .as_object_mut()
        .ok_or_else(|| AppError::state("Test config did not contain a recognition object."))?;
    recognition.remove("expectedLanguages");
    recognition.insert("languages".to_string(), serde_json::json!(["en"]));
    assert!(serde_json::from_value::<AppConfig>(legacy_language).is_err());

    let mut v3_stt = valid_config_json("openai/gpt-transcribe");
    v3_stt
        .as_object_mut()
        .ok_or_else(|| AppError::state("Test config was not an object."))?
        .insert(
            "stt".to_string(),
            serde_json::json!({
                "provider": "openai",
                "languages": ["en"],
                "model": "gpt-transcribe"
            }),
        );
    assert!(serde_json::from_value::<AppConfig>(v3_stt).is_err());
    Ok(())
}

#[test]
fn config_validation_rejects_empty_or_duplicate_language_hints() {
    let mut config = AppConfig::default();
    config.recognition.expected_languages = vec!["zh".to_string(), " ".to_string()];
    assert!(config.validate().is_err());

    config.recognition.expected_languages = vec!["en".to_string(), "EN".to_string()];
    assert!(config.validate().is_err());
}

fn valid_config_json(path: &str) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "audio": { "inputDeviceId": null },
        "recognition": {
            "path": path,
            "expectedLanguages": ["zh", "en"]
        },
        "osc": { "host": "127.0.0.1", "port": 9000, "enabled": true },
        "publication": { "mode": "completed" },
        "ui": { "showOngoingPreview": true }
    })
}
