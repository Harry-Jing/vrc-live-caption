use super::{
    AppError, AppResult, Matcher, authority_with_port, direct_matcher,
    matcher_for_configured_https_proxy,
};

#[cfg(target_os = "macos")]
pub(super) fn system_proxy_matcher(no_proxy: Option<String>) -> AppResult<Matcher> {
    use system_configuration::core_foundation::base::CFType;
    use system_configuration::core_foundation::dictionary::CFDictionary;
    use system_configuration::core_foundation::number::CFNumber;
    use system_configuration::core_foundation::string::CFString;
    use system_configuration::dynamic_store::SCDynamicStoreBuilder;

    fn number(proxies: &CFDictionary<CFString, CFType>, key: &str) -> AppResult<Option<i32>> {
        let key_value = CFString::new(key);
        let Some(value) = proxies.find(&key_value) else {
            return Ok(None);
        };
        value
            .downcast::<CFNumber>()
            .and_then(|number| number.to_i32())
            .map(Some)
            .ok_or_else(|| {
                AppError::stt_network(format!(
                    "macOS system proxy field {key} has an invalid type or value; refusing a direct OpenAI connection."
                ))
            })
    }

    fn string(proxies: &CFDictionary<CFString, CFType>, key: &str) -> AppResult<Option<String>> {
        let key_value = CFString::new(key);
        let Some(value) = proxies.find(&key_value) else {
            return Ok(None);
        };
        value
            .downcast::<CFString>()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| {
                AppError::stt_network(format!(
                    "macOS system proxy field {key} has an invalid type; refusing a direct OpenAI connection."
                ))
            })
    }

    let store = SCDynamicStoreBuilder::new("vrc-live-caption")
        .build()
        .ok_or_else(|| {
            AppError::stt_network(
                "macOS system proxy settings could not be opened; refusing a direct OpenAI connection.",
            )
        })?;
    let proxies = store.get_proxies().ok_or_else(|| {
        AppError::stt_network(
            "macOS system proxy settings could not be read; refusing a direct OpenAI connection.",
        )
    })?;
    let settings = MacProxySettings {
        https_enabled: number(&proxies, "HTTPSEnable")?.unwrap_or(0),
        https_host: string(&proxies, "HTTPSProxy")?,
        https_port: number(&proxies, "HTTPSPort")?,
        socks_enabled: number(&proxies, "SOCKSEnable")?.unwrap_or(0),
        pac_enabled: number(&proxies, "ProxyAutoConfigEnable")?.unwrap_or(0),
        auto_discovery_enabled: number(&proxies, "ProxyAutoDiscoveryEnable")?.unwrap_or(0),
    };
    matcher_for_settings(&settings, no_proxy.as_deref())
}

struct MacProxySettings {
    https_enabled: i32,
    https_host: Option<String>,
    https_port: Option<i32>,
    socks_enabled: i32,
    pac_enabled: i32,
    auto_discovery_enabled: i32,
}

fn matcher_for_settings(settings: &MacProxySettings, no_proxy: Option<&str>) -> AppResult<Matcher> {
    for (name, value) in [
        ("HTTPSEnable", settings.https_enabled),
        ("SOCKSEnable", settings.socks_enabled),
        ("ProxyAutoConfigEnable", settings.pac_enabled),
        ("ProxyAutoDiscoveryEnable", settings.auto_discovery_enabled),
    ] {
        if !matches!(value, 0 | 1) {
            return Err(AppError::stt_network(format!(
                "macOS system proxy field {name} has invalid value {value}; refusing a direct OpenAI connection."
            )));
        }
    }
    if settings.pac_enabled == 1 || settings.auto_discovery_enabled == 1 {
        return Err(AppError::stt_network(
            "macOS selected PAC or automatic proxy discovery, which is not supported for OpenAI Realtime; configure a manual HTTP CONNECT proxy.",
        ));
    }
    if settings.https_enabled == 0 {
        if settings.socks_enabled == 1 {
            return Err(AppError::stt_network(
                "macOS selected a SOCKS system proxy, which is not supported for OpenAI Realtime; configure a manual HTTP CONNECT proxy.",
            ));
        }
        return Ok(direct_matcher(no_proxy));
    }

    let host = settings
        .https_host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or_else(|| {
            AppError::stt_network(
                "macOS has an HTTPS proxy enabled but its host is missing; refusing a direct OpenAI connection.",
            )
        })?;
    let port = settings.https_port.ok_or_else(|| {
        AppError::stt_network(
            "macOS has an HTTPS proxy enabled but its port is missing; refusing a direct OpenAI connection.",
        )
    })?;
    let port = u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            AppError::stt_network(
                "macOS has an HTTPS proxy enabled but its port is invalid; refusing a direct OpenAI connection.",
            )
        })?;
    matcher_for_configured_https_proxy(&authority_with_port(host, port), no_proxy)
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;
