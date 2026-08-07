use super::*;
use crate::error::AppResult;

#[test]
fn handshake_request_uses_exact_model_url_and_bearer_auth_without_extra_intent() -> AppResult<()> {
    let api_key = SecretString::from("test-api-key".to_string());
    let request = openai_websocket_request(OpenAiTranscriptionModel::GptTranscribe, &api_key)?;

    assert_eq!(
        request.uri().to_string(),
        "wss://api.openai.com/v1/realtime?model=gpt-transcribe"
    );
    let authorization = request
        .headers()
        .get("Authorization")
        .ok_or_else(|| AppError::state("WebSocket request did not include Authorization."))?
        .to_str()
        .map_err(|error| AppError::state(format!("Invalid test Authorization header: {error}")))?;
    assert_eq!(authorization, "Bearer test-api-key");
    assert!(!request.uri().to_string().contains("intent"));
    assert!(request.headers().get("OpenAI-Beta").is_none());
    Ok(())
}

#[test]
fn empty_api_key_is_rejected_before_any_network_connection() {
    let api_key = SecretString::from("   ".to_string());
    assert!(
        openai_websocket_request(OpenAiTranscriptionModel::GptLiveTranscribe, &api_key).is_err()
    );
}

#[test]
fn handshake_http_statuses_produce_actionable_error_categories() {
    let auth_error = map_handshake_http_status(401);
    assert_eq!(auth_error.code(), "config.secret_failed");
    assert!(auth_error.to_string().contains("API key or project access"));

    let rate_error = map_handshake_http_status(429);
    assert_eq!(rate_error.code(), "stt.failed");
    assert!(rate_error.to_string().contains("rate or usage limit"));

    let provider_error = map_handshake_http_status(503);
    assert_eq!(provider_error.code(), "stt.failed");
    assert!(
        provider_error
            .to_string()
            .contains("temporarily unavailable")
    );
}
