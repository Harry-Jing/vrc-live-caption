use super::{AppError, AppResult, Matcher, direct_matcher, matcher_for_configured_https_proxy};

mod auto_detect;

#[cfg(target_os = "windows")]
use auto_detect::AutoProxyDetector;
use auto_detect::{AutoProxyDetection, ProbeOutcome};

/// windows-result reports a failed Win32 call as
/// `HRESULT_FROM_WIN32(GetLastError())`.
const fn hresult_from_win32(code: u16) -> i32 {
    (0x8007_0000_u32 | code as u32).cast_signed()
}

const HRESULT_FILE_NOT_FOUND: i32 = hresult_from_win32(2);
/// `ERROR_WINHTTP_AUTODETECTION_FAILED`: discovery completed without a script.
const HRESULT_AUTODETECTION_FAILED: i32 = hresult_from_win32(12180);
/// `ERROR_NO_UNICODE_TRANSLATION`: a proxy setting is not valid UTF-16 text.
const HRESULT_NO_UNICODE_TRANSLATION: i32 = hresult_from_win32(1113);

const CONFIGURED_SCRIPT_UNSUPPORTED: &str = "Windows proxy settings use a setup script (PAC), which OpenAI connections do not support yet; turn off \"Use setup script\" in Windows proxy settings and set a manual HTTP proxy if this network needs one, or set HTTPS_PROXY to an HTTP proxy.";
const DETECTED_SCRIPT_UNSUPPORTED: &str = "Windows automatic proxy detection found a setup script (PAC), which OpenAI connections do not support yet; turn off \"Automatically detect settings\" in Windows proxy settings and set a manual HTTP proxy if this network needs one, or set HTTPS_PROXY to an HTTP proxy.";
const DETECTION_FAILED: &str = "Windows automatic proxy detection failed, so the proxy route for OpenAI connections is unknown; turn off \"Automatically detect settings\" in Windows proxy settings and set a manual HTTP proxy if this network needs one, or set HTTPS_PROXY to an HTTP proxy.";
const DETECTION_UNFINISHED: &str = "Windows automatic proxy detection did not finish in time; if this keeps happening, turn off \"Automatically detect settings\" in Windows proxy settings or set HTTPS_PROXY to an HTTP proxy.";

#[cfg(target_os = "windows")]
pub(super) fn system_proxy_matcher(
    no_proxy: Option<String>,
    deadline: std::time::Instant,
    is_cancelled: &dyn Fn() -> bool,
) -> AppResult<Matcher> {
    // This documented WinHTTP bridge reads the current active LAN/VPN
    // connection. The individual Internet Settings registry values do not
    // provide an equivalent, reliable WPAD signal.
    let settings = current_user_settings(current_user::read())?;

    matcher_for_settings(&settings, no_proxy.as_deref(), || {
        shared_detector().detect_until(deadline, is_cancelled)
    })
}

#[cfg(target_os = "windows")]
#[allow(
    unsafe_code,
    reason = "winhttp's safe wrapper reads settings that are not valid UTF-16 as unset, which would bypass a configured proxy"
)]
mod current_user {
    use super::{WindowsProxySettings, decoded_setting};
    use std::string::FromUtf16Error;
    use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WinHttpGetIEProxyConfigForCurrentUser,
    };
    use windows::core::PWSTR;

    /// Reads the current user's proxy settings, or the HRESULT that stops them
    /// from being read.
    pub(super) fn read() -> Result<WindowsProxySettings, i32> {
        let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
        // SAFETY: `config` is a live, writable structure for the whole call.
        unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) }
            .map_err(|error| error.code().0)?;

        // Every string is taken before any is decoded, so all of them are freed
        // even when an earlier one is not valid text.
        // SAFETY: WinHTTP returned these strings for the caller to free, and each
        // pointer is taken exactly once.
        let (proxy_server, proxy_override, auto_config_url) = unsafe {
            (
                take(config.lpszProxy),
                take(config.lpszProxyBypass),
                take(config.lpszAutoConfigUrl),
            )
        };
        Ok(WindowsProxySettings {
            proxy_server: decoded_setting(proxy_server)?,
            proxy_override: decoded_setting(proxy_override)?,
            auto_config_url: decoded_setting(auto_config_url)?,
            auto_detect: config.fAutoDetect.as_bool(),
        })
    }

    /// Copies a string that WinHTTP returned, then frees it.
    ///
    /// # Safety
    ///
    /// `setting` must be null or a NUL-terminated string that WinHTTP allocated
    /// with `GlobalAlloc` and that nothing else frees.
    unsafe fn take(setting: PWSTR) -> Option<Result<String, FromUtf16Error>> {
        if setting.is_null() {
            return None;
        }
        // SAFETY: the caller guarantees a NUL-terminated string from WinHTTP.
        let text = unsafe { setting.to_string() };
        // SAFETY: WinHTTP allocated the string with GlobalAlloc, and this is its
        // only release. A failed release only leaks the string.
        let _ = unsafe { GlobalFree(Some(HGLOBAL(setting.as_ptr().cast()))) };
        Some(text)
    }
}

/// One worker and one reusable answer serve every connection in the process.
#[cfg(target_os = "windows")]
fn shared_detector() -> &'static AutoProxyDetector {
    static DETECTOR: std::sync::OnceLock<AutoProxyDetector> = std::sync::OnceLock::new();
    DETECTOR.get_or_init(|| AutoProxyDetector::new(probe_for_proxy_script))
}

/// Discovers a PAC script through DHCP and DNS, the WPAD sources behind
/// "Automatically detect settings". The call blocks and cannot be cancelled.
#[cfg(target_os = "windows")]
fn probe_for_proxy_script() -> ProbeOutcome {
    match winhttp::detect_auto_proxy_config_url() {
        Ok(_) => ProbeOutcome::ScriptFound,
        Err(error) => probe_outcome_for_error(error.code().0),
    }
}

fn probe_outcome_for_error(hresult: i32) -> ProbeOutcome {
    if hresult == HRESULT_AUTODETECTION_FAILED {
        ProbeOutcome::NoScript
    } else {
        ProbeOutcome::Failed
    }
}

struct WindowsProxySettings {
    proxy_server: Option<String>,
    proxy_override: Option<String>,
    auto_config_url: Option<String>,
    auto_detect: bool,
}

/// A setting that is not valid UTF-16 leaves the selected route unknown, so it
/// fails closed like an unreadable call instead of reading as unset.
fn decoded_setting(
    setting: Option<Result<String, std::string::FromUtf16Error>>,
) -> Result<Option<String>, i32> {
    setting
        .transpose()
        .map_err(|_| HRESULT_NO_UNICODE_TRANSLATION)
}

/// `WinHttpGetIEProxyConfigForCurrentUser` reports `ERROR_FILE_NOT_FOUND`
/// when the user has no Internet Settings. Microsoft's WinHTTP proxy sample
/// then assumes automatic detection with nothing else configured, which is
/// also the Windows default.
fn current_user_settings(
    read: Result<WindowsProxySettings, i32>,
) -> AppResult<WindowsProxySettings> {
    match read {
        Ok(settings) => Ok(settings),
        Err(HRESULT_FILE_NOT_FOUND) => Ok(WindowsProxySettings {
            proxy_server: None,
            proxy_override: None,
            auto_config_url: None,
            auto_detect: true,
        }),
        Err(hresult) => Err(AppError::recognition_network_terminal(format!(
            "Windows current-connection proxy settings could not be read (error 0x{:08X}); refusing a direct OpenAI connection.",
            hresult.cast_unsigned()
        ))),
    }
}

/// Follows WinINet's order: automatic detection, then the setup script, then
/// the manual proxy, then a direct connection. Detection that finds no script
/// is not a selected route, while any script is selected but unsupported.
fn matcher_for_settings(
    settings: &WindowsProxySettings,
    no_proxy: Option<&str>,
    detect_script: impl FnOnce() -> AutoProxyDetection,
) -> AppResult<Matcher> {
    let configured_script = settings
        .auto_config_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty());
    if configured_script {
        // Every detection outcome would still end at a script or a failure,
        // so fail before paying for discovery.
        return Err(AppError::recognition_network_terminal(
            CONFIGURED_SCRIPT_UNSUPPORTED,
        ));
    }
    if settings.auto_detect {
        route_after_detection(detect_script())?;
    }
    let Some(proxy_server) = settings.proxy_server.as_deref() else {
        return Ok(direct_matcher(no_proxy));
    };

    let no_proxy = no_proxy.or(settings.proxy_override.as_deref());
    matcher_for_proxy_server(proxy_server, no_proxy)
}

fn route_after_detection(detection: AutoProxyDetection) -> AppResult<()> {
    match detection {
        AutoProxyDetection::NoScript => Ok(()),
        AutoProxyDetection::ScriptFound => Err(AppError::recognition_network_terminal(
            DETECTED_SCRIPT_UNSUPPORTED,
        )),
        AutoProxyDetection::Failed => Err(AppError::recognition_network_terminal(DETECTION_FAILED)),
        // Discovery keeps running after the caller stops waiting, and a
        // "no script" answer is reused, so a later attempt can still succeed.
        AutoProxyDetection::Unfinished => Err(AppError::recognition_network_retryable(
            DETECTION_UNFINISHED,
        )),
        AutoProxyDetection::Cancelled => Err(AppError::recognition_network_terminal(
            "Proxy route selection was cancelled.",
        )),
    }
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
        return Err(AppError::recognition_network_terminal(
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
            AppError::recognition_network_terminal(format!(
                "Windows ProxyServer contains an invalid protocol entry: {entry}."
            ))
        })?;
        let proxy = proxy.trim();
        if proxy.is_empty() {
            return Err(AppError::recognition_network_terminal(format!(
                "Windows ProxyServer has an empty {protocol} proxy address."
            )));
        }
        match protocol.trim().to_ascii_lowercase().as_str() {
            "https" => {
                if https_proxy.replace(proxy.to_string()).is_some() {
                    return Err(AppError::recognition_network_terminal(
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
        return Err(AppError::recognition_network_terminal(
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
