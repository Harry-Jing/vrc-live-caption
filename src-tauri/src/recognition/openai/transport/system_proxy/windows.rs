use super::{AppError, AppResult, Matcher, direct_matcher, matcher_for_configured_https_proxy};

#[cfg(target_os = "windows")]
pub(super) fn system_proxy_matcher(no_proxy: Option<String>) -> AppResult<Matcher> {
    // This documented WinHTTP bridge reads the current active LAN/VPN
    // connection. The individual Internet Settings registry values do not
    // provide an equivalent, reliable WPAD signal.
    let config = winhttp::get_ie_proxy_config().map_err(|error| {
        AppError::stt_network_terminal(format!(
            "Windows current-connection proxy settings could not be read; refusing a direct OpenAI connection: {error}"
        ))
    })?;

    matcher_for_settings(
        &WindowsProxySettings {
            proxy_server: config.proxy,
            proxy_override: config.proxy_bypass,
            auto_config_url: config.auto_config_url,
            auto_detect: config.auto_detect,
        },
        no_proxy.as_deref(),
    )
}

struct WindowsProxySettings {
    proxy_server: Option<String>,
    proxy_override: Option<String>,
    auto_config_url: Option<String>,
    auto_detect: bool,
}

fn matcher_for_settings(
    settings: &WindowsProxySettings,
    no_proxy: Option<&str>,
) -> AppResult<Matcher> {
    let selected_pac = settings
        .auto_config_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty());
    if selected_pac || settings.auto_detect {
        return Err(AppError::stt_network_terminal(
            "Windows selected PAC or automatic proxy discovery, which is not supported for OpenAI Realtime; configure a manual HTTP CONNECT proxy.",
        ));
    }
    let Some(proxy_server) = settings.proxy_server.as_deref() else {
        return Ok(direct_matcher(no_proxy));
    };

    let no_proxy = no_proxy.or(settings.proxy_override.as_deref());
    matcher_for_proxy_server(proxy_server, no_proxy)
}

fn matcher_for_proxy_server(proxy_server: &str, no_proxy: Option<&str>) -> AppResult<Matcher> {
    let Some(proxy) = https_proxy(proxy_server)? else {
        return Ok(direct_matcher(no_proxy));
    };
    matcher_for_configured_https_proxy(&proxy, no_proxy)
}

fn https_proxy(proxy_server: &str) -> AppResult<Option<String>> {
    let proxy_server = proxy_server.trim();
    if proxy_server.is_empty() {
        return Err(AppError::stt_network_terminal(
            "Windows has a system proxy enabled but ProxyServer is empty.",
        ));
    }
    let is_protocol_map = proxy_list_entries(proxy_server).any(|entry| {
        entry
            .split_once('=')
            .is_some_and(|(protocol, _)| !protocol.contains("://"))
    });
    if !is_protocol_map {
        return Ok(Some(proxy_server.to_string()));
    }

    let mut https_proxy = None;
    let mut socks_proxy = None;
    for entry in proxy_list_entries(proxy_server) {
        let (protocol, proxy) = entry.split_once('=').ok_or_else(|| {
            AppError::stt_network_terminal(format!(
                "Windows ProxyServer contains an invalid protocol entry: {entry}."
            ))
        })?;
        let proxy = proxy.trim();
        if proxy.is_empty() {
            return Err(AppError::stt_network_terminal(format!(
                "Windows ProxyServer has an empty {protocol} proxy address."
            )));
        }
        match protocol.trim().to_ascii_lowercase().as_str() {
            "https" => {
                if https_proxy.replace(proxy.to_string()).is_some() {
                    return Err(AppError::stt_network_terminal(
                        "Windows ProxyServer contains more than one HTTPS proxy.",
                    ));
                }
            }
            "socks" => socks_proxy = Some(proxy.to_string()),
            _ => {}
        }
    }
    if let Some(proxy) = https_proxy {
        return Ok(Some(proxy));
    }
    if socks_proxy.is_some() {
        return Err(AppError::stt_network_terminal(
            "Windows selected a SOCKS system proxy, which is not supported for OpenAI Realtime; use an HTTP CONNECT proxy.",
        ));
    }
    Ok(None)
}

fn proxy_list_entries(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|character: char| character == ';' || character.is_ascii_whitespace())
        .filter(|entry| !entry.is_empty())
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
