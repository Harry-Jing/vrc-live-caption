use super::*;

#[test]
fn default_config_selects_gpt_transcribe_with_language_hints() {
    let config = AppConfig::default();

    assert_eq!(config.schema_version, 3);
    assert_eq!(config.stt.provider, SttProvider::OpenAi);
    assert_eq!(config.stt.model, OpenAiTranscriptionModel::GptTranscribe);
    assert_eq!(config.stt.languages, vec!["en"]);
}

#[test]
fn config_accepts_only_the_two_openai_transcription_models() -> AppResult<()> {
    for model in ["gpt-transcribe", "gpt-live-transcribe"] {
        let payload = valid_config_json(model);
        let config: AppConfig = serde_json::from_value(payload).map_err(|error| {
            AppError::state(format!("Supported model failed to deserialize: {error}"))
        })?;

        assert_eq!(config.stt.model.as_str(), model);
    }

    let legacy = valid_config_json("gpt-4o-mini-transcribe");
    assert!(serde_json::from_value::<AppConfig>(legacy).is_err());
    Ok(())
}

#[test]
fn config_rejects_the_legacy_language_field_and_mock_provider() -> AppResult<()> {
    let mut legacy_language = valid_config_json("gpt-transcribe");
    let stt = legacy_language["stt"]
        .as_object_mut()
        .ok_or_else(|| AppError::state("Test config did not contain an STT object."))?;
    stt.remove("languages");
    stt.insert("language".to_string(), serde_json::json!("en"));
    assert!(serde_json::from_value::<AppConfig>(legacy_language).is_err());

    let mut mock = valid_config_json("gpt-transcribe");
    mock["stt"]["provider"] = serde_json::json!("mock");
    assert!(serde_json::from_value::<AppConfig>(mock).is_err());
    Ok(())
}

#[test]
fn config_validation_rejects_empty_or_duplicate_language_hints() {
    let mut config = AppConfig::default();
    config.stt.languages = vec!["zh".to_string(), " ".to_string()];
    assert!(config.validate().is_err());

    config.stt.languages = vec!["en".to_string(), "EN".to_string()];
    assert!(config.validate().is_err());
}

fn valid_config_json(model: &str) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 3,
        "audio": { "inputDeviceId": null },
        "stt": {
            "provider": "openai",
            "languages": ["zh", "en"],
            "model": model
        },
        "osc": { "host": "127.0.0.1", "port": 9000, "enabled": true },
        "publication": { "mode": "completed" },
        "ui": { "showPartial": true }
    })
}
