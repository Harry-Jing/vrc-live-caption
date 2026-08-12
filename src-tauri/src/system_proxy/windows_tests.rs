use super::*;
use tungstenite::http::Uri;

#[test]
fn protocol_map_selects_https_without_silent_direct_fallback() -> AppResult<()> {
    let matcher = matcher_for_proxy_server(
        "http=plain-proxy.example:8080;https=secure-proxy.example:8443",
        None,
    )?;
    let target = "https://api.openai.com/"
        .parse::<Uri>()
        .map_err(|error| AppError::state(format!("Failed to parse test URI: {error}")))?;
    let selected = matcher
        .intercept(&target)
        .ok_or_else(|| AppError::state("The Windows HTTPS proxy map was treated as direct."))?;

    assert_eq!(selected.uri().scheme_str(), Some("http"));
    assert_eq!(selected.uri().host(), Some("secure-proxy.example"));
    assert_eq!(selected.uri().port_u16(), Some(8443));
    Ok(())
}

#[test]
fn whitespace_separated_protocol_map_selects_https() -> AppResult<()> {
    let matcher = matcher_for_proxy_server(
        "http=plain-proxy.example:8080\thttps=secure-proxy.example:8443",
        None,
    )?;
    let target = "https://api.openai.com/"
        .parse::<Uri>()
        .map_err(|error| AppError::state(format!("Failed to parse test URI: {error}")))?;
    let selected = matcher.intercept(&target).ok_or_else(|| {
        AppError::state("A whitespace-separated Windows HTTPS proxy map was treated as direct.")
    })?;

    assert_eq!(selected.uri().scheme_str(), Some("http"));
    assert_eq!(selected.uri().host(), Some("secure-proxy.example"));
    assert_eq!(selected.uri().port_u16(), Some(8443));
    Ok(())
}

#[test]
fn invalid_https_proxy_map_is_rejected() -> AppResult<()> {
    let error = matcher_for_proxy_server("http=proxy.example:8080;https=%%%", None)
        .err()
        .ok_or_else(|| AppError::state("An invalid Windows HTTPS proxy was treated as direct."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("proxy address is invalid"));
    Ok(())
}

#[test]
fn pac_selection_is_rejected_before_direct_routing() -> AppResult<()> {
    let error = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: None,
            proxy_override: None,
            auto_config_url: Some("http://proxy.example/proxy.pac".to_string()),
            auto_detect: false,
        },
        None,
    )
    .err()
    .ok_or_else(|| AppError::state("A Windows PAC selection was treated as direct."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("PAC"));
    Ok(())
}

#[test]
fn wpad_selection_is_rejected_before_direct_routing() -> AppResult<()> {
    let error = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: None,
            proxy_override: None,
            auto_config_url: None,
            auto_detect: true,
        },
        None,
    )
    .err()
    .ok_or_else(|| AppError::state("A Windows WPAD selection was treated as direct."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("automatic proxy discovery"));
    Ok(())
}

#[test]
fn automatic_selection_is_rejected_even_with_a_manual_proxy_present() -> AppResult<()> {
    let error = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: Some("manual-proxy.example:8080".to_string()),
            proxy_override: None,
            auto_config_url: Some("http://proxy.example/proxy.pac".to_string()),
            auto_detect: false,
        },
        None,
    )
    .err()
    .ok_or_else(|| AppError::state("A selected Windows PAC was bypassed by a manual proxy."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("PAC"));
    Ok(())
}

#[test]
fn http_only_protocol_map_keeps_https_direct() -> AppResult<()> {
    let matcher = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: Some("http=plain-proxy.example:8080".to_string()),
            proxy_override: None,
            auto_config_url: None,
            auto_detect: false,
        },
        None,
    )?;
    let target = "https://api.openai.com/"
        .parse::<Uri>()
        .map_err(|error| AppError::state(format!("Failed to parse test URI: {error}")))?;

    assert!(matcher.intercept(&target).is_none());
    Ok(())
}
