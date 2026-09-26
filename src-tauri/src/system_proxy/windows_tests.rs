use super::*;
use crate::error::RetryDisposition;
use std::cell::Cell;
use tungstenite::http::Uri;

fn openai_target() -> AppResult<Uri> {
    "https://api.openai.com/"
        .parse::<Uri>()
        .map_err(|error| AppError::state(format!("Failed to parse test URI: {error}")))
}

/// Records a detection that the settings should have made unnecessary.
fn unexpected_detection(ran: &Cell<bool>) -> impl FnOnce() -> AutoProxyDetection + '_ {
    move || {
        ran.set(true);
        AutoProxyDetection::Failed
    }
}

/// A manual proxy is configured too, so an answer that is not "no script"
/// must fail closed instead of falling through to it.
fn route_failure_after_detection(detection: AutoProxyDetection) -> AppResult<AppError> {
    matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: Some("manual-proxy.example:8080".to_string()),
            proxy_override: None,
            auto_config_url: None,
            auto_detect: true,
        },
        None,
        || detection,
    )
    .err()
    .ok_or_else(|| {
        AppError::state(format!(
            "Detection answer {detection:?} fell through to the manual proxy."
        ))
    })
}

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
    let detection_ran = Cell::new(false);
    let error = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: None,
            proxy_override: None,
            auto_config_url: Some("http://proxy.example/proxy.pac".to_string()),
            auto_detect: false,
        },
        None,
        unexpected_detection(&detection_ran),
    )
    .err()
    .ok_or_else(|| AppError::state("A Windows PAC selection was treated as direct."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert!(error.to_string().contains("PAC"));
    assert!(error.to_string().contains("Use setup script"));
    assert!(!detection_ran.get());
    Ok(())
}

#[test]
fn configured_pac_fails_closed_without_waiting_for_detection() -> AppResult<()> {
    let detection_ran = Cell::new(false);
    let error = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: None,
            proxy_override: None,
            auto_config_url: Some("http://proxy.example/proxy.pac".to_string()),
            auto_detect: true,
        },
        None,
        unexpected_detection(&detection_ran),
    )
    .err()
    .ok_or_else(|| AppError::state("A configured PAC was bypassed by automatic detection."))?;

    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert!(error.to_string().contains("PAC"));
    assert!(!detection_ran.get());
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
        || AutoProxyDetection::NoScript,
    )
    .err()
    .ok_or_else(|| AppError::state("A selected Windows PAC was bypassed by a manual proxy."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("PAC"));
    Ok(())
}

#[test]
fn detection_without_a_script_connects_directly() -> AppResult<()> {
    let matcher = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: None,
            proxy_override: None,
            auto_config_url: None,
            auto_detect: true,
        },
        None,
        || AutoProxyDetection::NoScript,
    )?;

    assert!(matcher.intercept(&openai_target()?).is_none());
    Ok(())
}

#[test]
fn detection_without_a_script_falls_through_to_the_manual_proxy() -> AppResult<()> {
    let matcher = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: Some("manual-proxy.example:8080".to_string()),
            proxy_override: None,
            auto_config_url: None,
            auto_detect: true,
        },
        None,
        || AutoProxyDetection::NoScript,
    )?;
    let selected = matcher.intercept(&openai_target()?).ok_or_else(|| {
        AppError::state("The manual proxy was bypassed after detection found no script.")
    })?;

    assert_eq!(selected.uri().scheme_str(), Some("http"));
    assert_eq!(selected.uri().host(), Some("manual-proxy.example"));
    assert_eq!(selected.uri().port_u16(), Some(8080));
    Ok(())
}

#[test]
fn manual_settings_do_not_run_detection() -> AppResult<()> {
    let detection_ran = Cell::new(false);
    let matcher = matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: Some("manual-proxy.example:8080".to_string()),
            proxy_override: None,
            auto_config_url: None,
            auto_detect: false,
        },
        None,
        unexpected_detection(&detection_ran),
    )?;

    assert!(matcher.intercept(&openai_target()?).is_some());
    assert!(!detection_ran.get());
    Ok(())
}

#[test]
fn detected_pac_fails_closed_with_actionable_guidance() -> AppResult<()> {
    let error = route_failure_after_detection(AutoProxyDetection::ScriptFound)?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert!(error.to_string().contains("PAC"));
    assert!(error.to_string().contains("Automatically detect settings"));
    assert!(error.to_string().contains("HTTPS_PROXY"));
    Ok(())
}

#[test]
fn unfinished_detection_is_retryable() -> AppResult<()> {
    let error = route_failure_after_detection(AutoProxyDetection::Unfinished)?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert_eq!(error.retry_disposition(), RetryDisposition::Retryable);
    Ok(())
}

#[test]
fn failed_detection_fails_closed_with_actionable_guidance() -> AppResult<()> {
    let error = route_failure_after_detection(AutoProxyDetection::Failed)?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert!(error.to_string().contains("Automatically detect settings"));
    assert!(error.to_string().contains("HTTPS_PROXY"));
    Ok(())
}

#[test]
fn cancelled_detection_selects_no_route() -> AppResult<()> {
    let error = route_failure_after_detection(AutoProxyDetection::Cancelled)?;

    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    Ok(())
}

#[test]
fn missing_internet_settings_mean_automatic_detection_only() -> AppResult<()> {
    let settings = current_user_settings(Err(HRESULT_FILE_NOT_FOUND))?;

    assert!(settings.auto_detect);
    assert!(settings.proxy_server.is_none());
    assert!(settings.proxy_override.is_none());
    assert!(settings.auto_config_url.is_none());
    let matcher = matcher_for_settings(&settings, None, || AutoProxyDetection::NoScript)?;
    assert!(matcher.intercept(&openai_target()?).is_none());
    Ok(())
}

#[test]
fn unreadable_internet_settings_fail_closed() -> AppResult<()> {
    let error_not_enough_memory = hresult_from_win32(8);
    let error = current_user_settings(Err(error_not_enough_memory))
        .err()
        .ok_or_else(|| AppError::state("Unreadable Windows proxy settings were accepted."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert!(error.to_string().contains("0x80070008"));
    Ok(())
}

#[test]
fn win32_errors_match_their_documented_hresults() {
    assert_eq!(HRESULT_FILE_NOT_FOUND, 0x8007_0002_u32.cast_signed());
    assert_eq!(HRESULT_AUTODETECTION_FAILED, 0x8007_2F94_u32.cast_signed());
}

#[test]
fn only_autodetection_failure_means_no_script() {
    let error_winhttp_internal_error = hresult_from_win32(12004);
    let e_fail = 0x8000_4005_u32.cast_signed();

    assert_eq!(
        probe_outcome_for_error(HRESULT_AUTODETECTION_FAILED),
        ProbeOutcome::NoScript
    );
    for hresult in [error_winhttp_internal_error, HRESULT_FILE_NOT_FOUND, e_fail] {
        assert_eq!(probe_outcome_for_error(hresult), ProbeOutcome::Failed);
    }
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
        || AutoProxyDetection::Failed,
    )?;
    let target = "https://api.openai.com/"
        .parse::<Uri>()
        .map_err(|error| AppError::state(format!("Failed to parse test URI: {error}")))?;

    assert!(matcher.intercept(&target).is_none());
    Ok(())
}
