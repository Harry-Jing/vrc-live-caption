use super::{
    AppError, AppResult, Matcher, authority_with_port, direct_matcher,
    matcher_for_configured_https_proxy,
};

#[cfg(target_os = "macos")]
use system_configuration::core_foundation::{
    base::CFType, dictionary::CFDictionary, number::CFNumber, string::CFString,
};

#[cfg(target_os = "macos")]
type MacProxyDictionary = CFDictionary<CFString, CFType>;

#[cfg(target_os = "macos")]
pub(super) fn system_proxy_matcher(target_url: &str) -> AppResult<Matcher> {
    use system_configuration::dynamic_store::SCDynamicStoreBuilder;

    let store = SCDynamicStoreBuilder::new("vrc-live-caption")
        .build()
        .ok_or_else(|| {
            AppError::stt_network_terminal(
                "macOS system proxy settings could not be opened; refusing a direct OpenAI connection.",
            )
        })?;
    let proxies = store.get_proxies().ok_or_else(|| {
        AppError::stt_network_terminal(
            "macOS system proxy settings could not be read; refusing a direct OpenAI connection.",
        )
    })?;

    match route_for(target_url, &proxies)? {
        MacProxyRoute::Direct => Ok(direct_matcher(None)),
        MacProxyRoute::HttpConnect(proxy) => matcher_for_configured_https_proxy(&proxy, None),
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, PartialEq, Eq)]
enum MacProxyRoute {
    Direct,
    HttpConnect(String),
}

#[cfg(target_os = "macos")]
fn route_for(target_url: &str, proxies: &MacProxyDictionary) -> AppResult<MacProxyRoute> {
    let settings = settings_from_dictionary(proxies)?;
    let configured_proxy = configured_proxy_for_settings(&settings)?;
    if configured_proxy.is_none() {
        return Ok(MacProxyRoute::Direct);
    }

    match cfnetwork::first_proxy_for_url(target_url, proxies)? {
        cfnetwork::ResolvedProxy::Direct => Ok(MacProxyRoute::Direct),
        cfnetwork::ResolvedProxy::HttpConnect { host, port } => {
            Ok(MacProxyRoute::HttpConnect(authority_with_port(&host, port)))
        }
        cfnetwork::ResolvedProxy::Unsupported(proxy_type) => {
            Err(AppError::stt_network_terminal(format!(
                "macOS selected unsupported proxy type '{proxy_type}' for OpenAI Realtime; configure a manual HTTP CONNECT proxy."
            )))
        }
    }
}

#[cfg(target_os = "macos")]
fn settings_from_dictionary(proxies: &MacProxyDictionary) -> AppResult<MacProxySettings> {
    let exceptions_key = CFString::new("ExceptionsList");
    if let Some(exceptions) = proxies.find(&exceptions_key)
        && !cfnetwork::is_string_array(&exceptions)
    {
        return Err(AppError::stt_network_terminal(
            "macOS system proxy field ExceptionsList must be an array containing only strings; refusing a direct OpenAI connection.",
        ));
    }

    if let Some(value) = number(proxies, "ExcludeSimpleHostnames")?
        && !matches!(value, 0 | 1)
    {
        return Err(AppError::stt_network_terminal(format!(
            "macOS system proxy field ExcludeSimpleHostnames has invalid value {value}; refusing a direct OpenAI connection."
        )));
    }

    Ok(MacProxySettings {
        https_enabled: number(proxies, "HTTPSEnable")?.unwrap_or(0),
        https_host: string(proxies, "HTTPSProxy")?,
        https_port: number(proxies, "HTTPSPort")?,
        socks_enabled: number(proxies, "SOCKSEnable")?.unwrap_or(0),
        pac_enabled: number(proxies, "ProxyAutoConfigEnable")?.unwrap_or(0),
        auto_discovery_enabled: number(proxies, "ProxyAutoDiscoveryEnable")?.unwrap_or(0),
    })
}

#[cfg(target_os = "macos")]
fn number(proxies: &MacProxyDictionary, key: &str) -> AppResult<Option<i32>> {
    let key_value = CFString::new(key);
    let Some(value) = proxies.find(&key_value) else {
        return Ok(None);
    };
    value
        .downcast::<CFNumber>()
        .and_then(|number| number.to_i32())
        .map(Some)
        .ok_or_else(|| {
            AppError::stt_network_terminal(format!(
                "macOS system proxy field {key} has an invalid type or value; refusing a direct OpenAI connection."
            ))
        })
}

#[cfg(target_os = "macos")]
fn string(proxies: &MacProxyDictionary, key: &str) -> AppResult<Option<String>> {
    let key_value = CFString::new(key);
    let Some(value) = proxies.find(&key_value) else {
        return Ok(None);
    };
    value
        .downcast::<CFString>()
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| {
            AppError::stt_network_terminal(format!(
                "macOS system proxy field {key} has an invalid type; refusing a direct OpenAI connection."
            ))
        })
}

struct MacProxySettings {
    https_enabled: i32,
    https_host: Option<String>,
    https_port: Option<i32>,
    socks_enabled: i32,
    pac_enabled: i32,
    auto_discovery_enabled: i32,
}

#[cfg(test)]
fn matcher_for_settings(settings: &MacProxySettings, no_proxy: Option<&str>) -> AppResult<Matcher> {
    let Some(proxy) = configured_proxy_for_settings(settings)? else {
        return Ok(direct_matcher(no_proxy));
    };
    matcher_for_configured_https_proxy(&proxy, no_proxy)
}

fn configured_proxy_for_settings(settings: &MacProxySettings) -> AppResult<Option<String>> {
    for (name, value) in [
        ("HTTPSEnable", settings.https_enabled),
        ("SOCKSEnable", settings.socks_enabled),
        ("ProxyAutoConfigEnable", settings.pac_enabled),
        ("ProxyAutoDiscoveryEnable", settings.auto_discovery_enabled),
    ] {
        if !matches!(value, 0 | 1) {
            return Err(AppError::stt_network_terminal(format!(
                "macOS system proxy field {name} has invalid value {value}; refusing a direct OpenAI connection."
            )));
        }
    }
    if settings.pac_enabled == 1 || settings.auto_discovery_enabled == 1 {
        return Err(AppError::stt_network_terminal(
            "macOS selected PAC or automatic proxy discovery, which is not supported for OpenAI Realtime; configure a manual HTTP CONNECT proxy.",
        ));
    }
    if settings.https_enabled == 0 {
        if settings.socks_enabled == 1 {
            return Err(AppError::stt_network_terminal(
                "macOS selected a SOCKS system proxy, which is not supported for OpenAI Realtime; configure a manual HTTP CONNECT proxy.",
            ));
        }
        return Ok(None);
    }

    let host = settings
        .https_host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or_else(|| {
            AppError::stt_network_terminal(
                "macOS has an HTTPS proxy enabled but its host is missing; refusing a direct OpenAI connection.",
            )
        })?;
    let port = settings.https_port.ok_or_else(|| {
        AppError::stt_network_terminal(
            "macOS has an HTTPS proxy enabled but its port is missing; refusing a direct OpenAI connection.",
        )
    })?;
    let port = u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            AppError::stt_network_terminal(
                "macOS has an HTTPS proxy enabled but its port is invalid; refusing a direct OpenAI connection.",
            )
        })?;
    Ok(Some(authority_with_port(host, port)))
}

#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "CFNetwork has no safe Rust binding compatible with system-configuration's Core Foundation types"
)]
mod cfnetwork {
    use super::{AppError, AppResult, MacProxyDictionary};
    use std::ptr;
    use system_configuration::core_foundation::{
        array::{CFArray, CFArrayRef},
        base::{CFType, TCFType},
        dictionary::{CFDictionary, CFDictionaryRef},
        number::CFNumber,
        string::{CFString, CFStringRef},
        url::{CFURL, CFURLCreateWithString, CFURLRef},
    };

    pub(super) enum ResolvedProxy {
        Direct,
        HttpConnect { host: String, port: u16 },
        Unsupported(String),
    }

    pub(super) fn is_string_array(value: &CFType) -> bool {
        let Some(array) = value.downcast::<CFArray>() else {
            return false;
        };
        // SAFETY: SystemConfiguration proxy dictionaries contain property-list
        // objects. Retyping the already type-checked CFArray as CFType elements
        // lets us validate every element before CFNetwork receives the array.
        let typed: CFArray<CFType> =
            unsafe { CFArray::wrap_under_get_rule(array.as_concrete_TypeRef()) };
        typed.iter().all(|entry| entry.instance_of::<CFString>())
    }

    #[link(name = "CFNetwork", kind = "framework")]
    unsafe extern "C" {
        fn CFNetworkCopyProxiesForURL(url: CFURLRef, proxy_settings: CFDictionaryRef)
        -> CFArrayRef;

        static kCFProxyTypeKey: CFStringRef;
        static kCFProxyHostNameKey: CFStringRef;
        static kCFProxyPortNumberKey: CFStringRef;
        static kCFProxyTypeNone: CFStringRef;
        static kCFProxyTypeHTTP: CFStringRef;
        static kCFProxyTypeHTTPS: CFStringRef;
    }

    pub(super) fn first_proxy_for_url(
        target_url: &str,
        settings: &MacProxyDictionary,
    ) -> AppResult<ResolvedProxy> {
        let target_string = CFString::new(target_url);
        // SAFETY: `target_string` is a live CFString, the base URL is null by
        // design, and a non-null create-rule result is immediately owned by
        // `CFURL`.
        let target_ref = unsafe {
            CFURLCreateWithString(
                ptr::null(),
                target_string.as_concrete_TypeRef(),
                ptr::null(),
            )
        };
        if target_ref.is_null() {
            return Err(AppError::stt_network_terminal(
                "The OpenAI Realtime target URL could not be represented for macOS proxy routing.",
            ));
        }
        // SAFETY: the null check above establishes a valid create-rule CFURL.
        let target = unsafe { CFURL::wrap_under_create_rule(target_ref) };

        // SAFETY: both arguments are live objects of the exact CF types required
        // by CFNetwork. The returned create-rule array is checked before owning it.
        let proxies_ref = unsafe {
            CFNetworkCopyProxiesForURL(target.as_concrete_TypeRef(), settings.as_concrete_TypeRef())
        };
        if proxies_ref.is_null() {
            return Err(AppError::stt_network_terminal(
                "macOS could not resolve system proxy routing for OpenAI Realtime; refusing a direct connection.",
            ));
        }
        // SAFETY: CFNetwork documents every element as a proxy dictionary and
        // returns this array under the create rule.
        let proxies: CFArray<CFDictionary<CFString, CFType>> =
            unsafe { CFArray::wrap_under_create_rule(proxies_ref) };
        let proxy = proxies.get(0).ok_or_else(|| {
            AppError::stt_network_terminal(
                "macOS returned no system proxy route for OpenAI Realtime; refusing a direct connection.",
            )
        })?;

        // SAFETY: these are process-lifetime CFString constants exported by
        // CFNetwork; get-rule wrappers retain them for the local value lifetime.
        let type_key = unsafe { CFString::wrap_under_get_rule(kCFProxyTypeKey) };
        let host_key = unsafe { CFString::wrap_under_get_rule(kCFProxyHostNameKey) };
        let port_key = unsafe { CFString::wrap_under_get_rule(kCFProxyPortNumberKey) };
        let direct_type = unsafe { CFString::wrap_under_get_rule(kCFProxyTypeNone) };
        let http_type = unsafe { CFString::wrap_under_get_rule(kCFProxyTypeHTTP) };
        let https_type = unsafe { CFString::wrap_under_get_rule(kCFProxyTypeHTTPS) };

        let proxy_type = proxy
            .find(&type_key)
            .and_then(|value| value.downcast::<CFString>())
            .ok_or_else(|| {
                AppError::stt_network_terminal(
                    "macOS returned a system proxy route without a valid type; refusing a direct connection.",
                )
            })?;
        if proxy_type == direct_type {
            return Ok(ResolvedProxy::Direct);
        }
        if proxy_type != http_type && proxy_type != https_type {
            return Ok(ResolvedProxy::Unsupported(proxy_type.to_string()));
        }

        let host = proxy
            .find(&host_key)
            .and_then(|value| value.downcast::<CFString>())
            .map(|value| value.to_string())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AppError::stt_network_terminal(
                    "macOS returned an HTTP CONNECT proxy without a valid host; refusing a direct connection.",
                )
            })?;
        let port = proxy
            .find(&port_key)
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|value| value.to_i32())
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value != 0)
            .ok_or_else(|| {
                AppError::stt_network_terminal(
                    "macOS returned an HTTP CONNECT proxy without a valid port; refusing a direct connection.",
                )
            })?;

        Ok(ResolvedProxy::HttpConnect { host, port })
    }
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;
