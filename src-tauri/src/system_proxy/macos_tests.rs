use super::*;

#[cfg(target_os = "macos")]
use system_configuration::core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};

#[cfg(target_os = "macos")]
#[test]
fn exact_system_exception_routes_the_openai_target_directly() -> AppResult<()> {
    let settings = synthetic_proxy_dictionary(&[
        ("HTTPSEnable", CFNumber::from(1_i32).as_CFType()),
        (
            "HTTPSProxy",
            CFString::new("manual-proxy.example").as_CFType(),
        ),
        ("HTTPSPort", CFNumber::from(8443_i32).as_CFType()),
        (
            "ExceptionsList",
            CFArray::from_CFTypes(&[CFString::new("api.openai.com")]).as_CFType(),
        ),
    ]);

    assert_eq!(
        route_for("https://api.openai.com/", &settings)?,
        MacProxyRoute::Direct
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn excluded_simple_hostname_routes_directly() -> AppResult<()> {
    let settings = synthetic_proxy_dictionary(&[
        ("HTTPSEnable", CFNumber::from(1_i32).as_CFType()),
        (
            "HTTPSProxy",
            CFString::new("manual-proxy.example").as_CFType(),
        ),
        ("HTTPSPort", CFNumber::from(8443_i32).as_CFType()),
        ("ExcludeSimpleHostnames", CFNumber::from(1_i32).as_CFType()),
    ]);

    assert_eq!(
        route_for("https://printer/", &settings)?,
        MacProxyRoute::Direct
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn malformed_exclude_simple_hostnames_is_rejected_before_routing() -> AppResult<()> {
    let settings = synthetic_proxy_dictionary(&[
        ("HTTPSEnable", CFNumber::from(1_i32).as_CFType()),
        (
            "HTTPSProxy",
            CFString::new("manual-proxy.example").as_CFType(),
        ),
        ("HTTPSPort", CFNumber::from(8443_i32).as_CFType()),
        ("ExcludeSimpleHostnames", CFString::new("yes").as_CFType()),
    ]);

    let error = route_for("https://api.openai.com/", &settings)
        .err()
        .ok_or_else(|| AppError::state("A malformed macOS bypass flag was accepted."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("ExcludeSimpleHostnames"));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn malformed_exceptions_list_is_rejected_before_routing() -> AppResult<()> {
    let settings = synthetic_proxy_dictionary(&[
        ("HTTPSEnable", CFNumber::from(1_i32).as_CFType()),
        (
            "HTTPSProxy",
            CFString::new("manual-proxy.example").as_CFType(),
        ),
        ("HTTPSPort", CFNumber::from(8443_i32).as_CFType()),
        (
            "ExceptionsList",
            CFString::new("api.openai.com").as_CFType(),
        ),
    ]);

    let error = route_for("https://api.openai.com/", &settings)
        .err()
        .ok_or_else(|| AppError::state("A malformed macOS exceptions list was accepted."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("ExceptionsList"));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn non_string_system_exception_is_rejected_before_routing() -> AppResult<()> {
    let settings = synthetic_proxy_dictionary(&[
        ("HTTPSEnable", CFNumber::from(1_i32).as_CFType()),
        (
            "HTTPSProxy",
            CFString::new("manual-proxy.example").as_CFType(),
        ),
        ("HTTPSPort", CFNumber::from(8443_i32).as_CFType()),
        (
            "ExceptionsList",
            CFArray::from_CFTypes(&[CFNumber::from(7_i32)]).as_CFType(),
        ),
    ]);

    let error = route_for("https://api.openai.com/", &settings)
        .err()
        .ok_or_else(|| AppError::state("A non-string macOS system exception was accepted."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains("ExceptionsList"));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn target_without_a_system_exception_uses_the_manual_proxy() -> AppResult<()> {
    let settings = synthetic_proxy_dictionary(&[
        ("HTTPSEnable", CFNumber::from(1_i32).as_CFType()),
        (
            "HTTPSProxy",
            CFString::new("manual-proxy.example").as_CFType(),
        ),
        ("HTTPSPort", CFNumber::from(8443_i32).as_CFType()),
        (
            "ExceptionsList",
            CFArray::from_CFTypes(&[CFString::new("internal.example")]).as_CFType(),
        ),
    ]);

    assert_eq!(
        route_for("https://api.openai.com/", &settings)?,
        MacProxyRoute::HttpConnect("manual-proxy.example:8443".to_string())
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn pac_is_rejected_before_target_resolution() -> AppResult<()> {
    assert_automatic_proxy_is_rejected_before_target_resolution("ProxyAutoConfigEnable", "PAC")
}

#[cfg(target_os = "macos")]
#[test]
fn wpad_is_rejected_before_target_resolution() -> AppResult<()> {
    assert_automatic_proxy_is_rejected_before_target_resolution(
        "ProxyAutoDiscoveryEnable",
        "automatic proxy discovery",
    )
}

#[cfg(target_os = "macos")]
fn assert_automatic_proxy_is_rejected_before_target_resolution(
    automatic_key: &str,
    expected_detail: &str,
) -> AppResult<()> {
    let settings = synthetic_proxy_dictionary(&[
        ("HTTPSEnable", CFNumber::from(1_i32).as_CFType()),
        (
            "HTTPSProxy",
            CFString::new("manual-proxy.example").as_CFType(),
        ),
        ("HTTPSPort", CFNumber::from(8443_i32).as_CFType()),
        (automatic_key, CFNumber::from(1_i32).as_CFType()),
    ]);

    // A malformed target would fail CFURL construction if route resolution
    // were reached. The selected automatic mode must fail first instead.
    let error = route_for("not a URL", &settings)
        .err()
        .ok_or_else(|| AppError::state("An automatic macOS proxy selection was accepted."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(error.to_string().contains(expected_detail));
    Ok(())
}

#[cfg(target_os = "macos")]
fn synthetic_proxy_dictionary(entries: &[(&str, CFType)]) -> CFDictionary<CFString, CFType> {
    let entries = entries
        .iter()
        .map(|(key, value)| (CFString::new(key), value.clone()))
        .collect::<Vec<_>>();
    CFDictionary::from_CFType_pairs(&entries)
}

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
