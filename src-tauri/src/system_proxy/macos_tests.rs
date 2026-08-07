use super::*;

#[test]
fn enabled_proxy_with_missing_host_is_invalid_instead_of_direct() -> AppResult<()> {
    let error = matcher_for_settings(
        &MacProxySettings {
            https_enabled: 1,
            https_host: None,
            https_port: Some(8080),
            socks_enabled: 0,
            pac_enabled: 0,
            auto_discovery_enabled: 0,
        },
        None,
    )
    .err()
    .ok_or_else(|| AppError::state("A malformed macOS proxy was treated as direct."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("host is missing"));
    Ok(())
}

#[test]
fn unsupported_automatic_proxy_is_rejected() -> AppResult<()> {
    let error = matcher_for_settings(
        &MacProxySettings {
            https_enabled: 0,
            https_host: None,
            https_port: None,
            socks_enabled: 0,
            pac_enabled: 1,
            auto_discovery_enabled: 0,
        },
        None,
    )
    .err()
    .ok_or_else(|| AppError::state("A macOS PAC proxy was treated as direct."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("PAC"));
    Ok(())
}
