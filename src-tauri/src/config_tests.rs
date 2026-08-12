use super::*;

#[test]
fn default_config_selects_the_gpt_transcribe_recognition_path_with_language_hints() {
    let config = AppConfig::default();

    assert_eq!(config.schema_version, 2);
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
fn config_rejects_pre_baseline_recognition_fields_in_the_current_v2_schema() -> AppResult<()> {
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

#[test]
fn config_validation_requires_translation_selection_for_translation_content() {
    for content in [
        ContentSelection::TranslationOnly,
        ContentSelection::Bilingual,
    ] {
        let mut config = AppConfig::default();
        config.publication.content = content;

        assert!(config.validate().is_err());
    }
}

#[test]
fn source_only_accepts_a_dormant_translation_selection() -> AppResult<()> {
    let mut value = valid_translation_config_json();
    value["publication"]["content"] = serde_json::json!("sourceOnly");
    let config: AppConfig = serde_json::from_value(value).map_err(|error| {
        AppError::state(format!(
            "Dormant translation selection failed to deserialize: {error}"
        ))
    })?;

    config.validate()
}

#[test]
fn config_accepts_only_the_phase_5_translation_vocabulary() -> AppResult<()> {
    for target in ["en", "zh-Hans"] {
        let mut value = valid_translation_config_json();
        value["translation"]["target"] = serde_json::json!(target);
        let config: AppConfig = serde_json::from_value(value).map_err(|error| {
            AppError::state(format!(
                "Supported translation target {target} failed to deserialize: {error}"
            ))
        })?;
        config.validate()?;
    }

    for content in ["sourceOnly", "translationOnly", "bilingual"] {
        let mut value = valid_translation_config_json();
        value["publication"]["content"] = serde_json::json!(content);
        let config: AppConfig = serde_json::from_value(value).map_err(|error| {
            AppError::state(format!(
                "Supported content selection {content} failed to deserialize: {error}"
            ))
        })?;
        config.validate()?;
    }

    for (pointer, unsupported) in [
        (
            "/translation/path",
            serde_json::json!("openai/chat-completions"),
        ),
        ("/translation/target", serde_json::json!("fr")),
        ("/publication/content", serde_json::json!("translated")),
        ("/translation/endpoint/kind", serde_json::json!("automatic")),
    ] {
        let mut value = valid_translation_config_json();
        let field = value
            .pointer_mut(pointer)
            .ok_or_else(|| AppError::state(format!("Test config is missing {pointer}.")))?;
        *field = unsupported;

        assert!(
            serde_json::from_value::<AppConfig>(value).is_err(),
            "unsupported translation vocabulary was accepted at {pointer}"
        );
    }

    Ok(())
}

#[test]
fn translation_config_and_endpoint_variants_require_exact_tagged_fields() -> AppResult<()> {
    let mut missing_path = valid_translation_config_json();
    translation_object_mut(&mut missing_path)?.remove("path");

    let mut missing_target = valid_translation_config_json();
    translation_object_mut(&mut missing_target)?.remove("target");

    let mut missing_endpoint = valid_translation_config_json();
    translation_object_mut(&mut missing_endpoint)?.remove("endpoint");

    let mut arbitrary_model = valid_translation_config_json();
    translation_object_mut(&mut arbitrary_model)?
        .insert("model".to_string(), serde_json::json!("arbitrary-model"));

    let mut official_with_custom_url = valid_translation_config_json();
    official_with_custom_url["translation"]["endpoint"] = serde_json::json!({
        "kind": "official",
        "apiBaseUrl": "https://example.com/v1"
    });

    let mut custom_without_url = valid_translation_config_json();
    custom_without_url["translation"]["endpoint"] = serde_json::json!({
        "kind": "custom"
    });

    let mut custom_with_unknown_field = valid_translation_config_json();
    custom_with_unknown_field["translation"]["endpoint"] = serde_json::json!({
        "kind": "custom",
        "apiBaseUrl": "https://example.com/v1",
        "fallback": "official"
    });

    let mut endpoint_without_kind = valid_translation_config_json();
    endpoint_without_kind["translation"]["endpoint"] = serde_json::json!({
        "apiBaseUrl": "https://example.com/v1"
    });

    for (label, value) in [
        ("missing path", missing_path),
        ("missing target", missing_target),
        ("missing endpoint", missing_endpoint),
        ("arbitrary model", arbitrary_model),
        (
            "official endpoint with custom URL",
            official_with_custom_url,
        ),
        ("custom endpoint without URL", custom_without_url),
        (
            "custom endpoint with unknown field",
            custom_with_unknown_field,
        ),
        ("endpoint without kind", endpoint_without_kind),
    ] {
        assert!(
            serde_json::from_value::<AppConfig>(value).is_err(),
            "translation config with {label} was accepted"
        );
    }

    Ok(())
}

#[test]
fn config_accepts_only_verified_custom_translation_api_base_urls() -> AppResult<()> {
    let mut valid = valid_translation_config_json();
    valid["translation"]["endpoint"] = serde_json::json!({
        "kind": "custom",
        "apiBaseUrl": "https://example.com/v1"
    });
    let config: AppConfig = serde_json::from_value(valid.clone()).map_err(|error| {
        AppError::state(format!(
            "Verified API base URL failed to deserialize: {error}"
        ))
    })?;
    config.validate()?;

    for invalid in [
        "http://example.com/v1",
        "https://user:secret@example.com/v1",
        "https://@example.com/v1",
        "https://example.com/v1?region=test",
        "https://example.com/v1?",
        "https://example.com/v1#responses",
        "https://example.com/v1#",
        "https://example.com/v1/responses",
        "https://example.com/v1/responses/",
        "https://example.com/v1/%72esponses",
        "https://example.com/v1/respon%73es",
        "https://example.com/v1/%",
        "not a URL",
    ] {
        let mut value = valid.clone();
        value["translation"]["endpoint"]["apiBaseUrl"] = serde_json::json!(invalid);
        let accepted = serde_json::from_value::<AppConfig>(value)
            .is_ok_and(|candidate| candidate.validate().is_ok());

        assert!(!accepted, "invalid API base URL was accepted: {invalid}");
    }

    Ok(())
}

fn valid_translation_config_json() -> serde_json::Value {
    let mut value = valid_config_json("openai/gpt-transcribe");
    value["translation"] = serde_json::json!({
        "path": "openai/responses-completed-text",
        "target": "zh-Hans",
        "endpoint": { "kind": "official" }
    });
    value["publication"]["content"] = serde_json::json!("translationOnly");
    value
}

fn translation_object_mut(
    value: &mut serde_json::Value,
) -> AppResult<&mut serde_json::Map<String, serde_json::Value>> {
    value["translation"]
        .as_object_mut()
        .ok_or_else(|| AppError::state("Test config is missing translation."))
}

fn valid_config_json(path: &str) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 2,
        "audio": { "inputDeviceId": null },
        "recognition": {
            "path": path,
            "expectedLanguages": ["zh", "en"]
        },
        "translation": null,
        "osc": { "host": "127.0.0.1", "port": 9000, "enabled": true },
        "publication": { "mode": "completed", "content": "sourceOnly" },
        "ui": { "showOngoingPreview": true }
    })
}
