use super::*;

#[test]
fn display_suffix_uses_at_most_last_four_characters() {
    assert_eq!(display_suffix("sk-test-abcdef"), Some("cdef".to_string()));
    assert_eq!(display_suffix("abc"), Some("abc".to_string()));
    assert_eq!(display_suffix(""), None);
}

#[test]
fn normalize_secret_rejects_empty_or_control_text() {
    assert!(normalize_secret("  ").is_err());
    assert!(normalize_secret("abc\ndef").is_err());
    assert!(normalize_secret(" sk-valid ").is_ok());
}

#[test]
fn test_credential_statuses_expose_both_ids_without_opening_the_real_store() {
    let statuses = credential_statuses();
    assert_eq!(
        statuses,
        vec![
            CredentialStatus::Unconfigured {
                id: CredentialId::OpenAi,
            },
            CredentialStatus::Unconfigured {
                id: CredentialId::CustomTranslation,
            },
        ]
    );
}

#[test]
fn custom_translation_credential_id_has_its_own_wire_value() {
    let value = serde_json::to_value(CredentialStatus::unconfigured(
        CredentialId::CustomTranslation,
    ))
    .unwrap_or_else(|error| serde_json::json!({ "serializationError": error.to_string() }));

    assert_eq!(
        value,
        serde_json::json!({
            "state": "unconfigured",
            "id": "customTranslation",
        })
    );
    assert_ne!(CredentialId::CustomTranslation, CredentialId::OpenAi);
    assert_eq!(
        CredentialStatus::unconfigured(CredentialId::OpenAi),
        CredentialStatus::Unconfigured {
            id: CredentialId::OpenAi,
        }
    );
}

#[test]
fn custom_translation_never_uses_the_openai_environment_fallback() {
    let environment_reads = std::cell::Cell::new(0_u8);
    for system_result in [
        Ok(None),
        Err(AppError::secret("custom credential store unavailable")),
    ] {
        let result = resolve_credential_from_sources(
            CredentialId::CustomTranslation,
            |_| system_result,
            || {
                environment_reads.set(environment_reads.get().saturating_add(1));
                Some(SecretString::from("openai-environment-secret"))
            },
        );

        assert!(result.is_err());
    }
    assert_eq!(environment_reads.get(), 0);
}

#[test]
fn credential_resolution_uses_the_endpoint_specific_source_policy() -> AppResult<()> {
    let official = resolve_credential_from_sources(
        CredentialId::OpenAi,
        |_| Ok(None),
        || Some(SecretString::from("official-environment-secret")),
    )?;
    assert_eq!(official.id, CredentialId::OpenAi);
    assert_eq!(official.storage, CredentialStorage::Environment);
    assert_eq!(
        official.secret.expose_secret(),
        "official-environment-secret"
    );

    let custom = resolve_credential_from_sources(
        CredentialId::CustomTranslation,
        |account| {
            assert_eq!(account, CUSTOM_TRANSLATION_ACCOUNT);
            Ok(Some(SecretString::from("custom-store-secret")))
        },
        || Some(SecretString::from("must-not-be-used")),
    )?;
    assert_eq!(custom.id, CredentialId::CustomTranslation);
    assert_eq!(custom.storage, CredentialStorage::SystemCredentialStore);
    assert_eq!(custom.secret.expose_secret(), "custom-store-secret");
    Ok(())
}

#[test]
fn unconfigured_status_serializes_without_configured_only_fields() {
    let value = serde_json::to_value(CredentialStatus::unconfigured(CredentialId::OpenAi))
        .unwrap_or_else(|error| serde_json::json!({ "serializationError": error.to_string() }));

    assert_eq!(
        value,
        serde_json::json!({
            "state": "unconfigured",
            "id": "openai",
        })
    );
}

#[test]
fn unavailable_status_serializes_the_stable_application_failure() {
    let value = serde_json::to_value(CredentialStatus::unavailable(
        CredentialId::OpenAi,
        &AppError::secret("System credential store is unavailable."),
    ))
    .unwrap_or_else(|error| serde_json::json!({ "serializationError": error.to_string() }));

    assert_eq!(
        value,
        serde_json::json!({
            "state": "unavailable",
            "id": "openai",
            "failure": {
                "code": "config.secret_failed",
                "message": "System credential store is unavailable.",
            },
        })
    );
}
