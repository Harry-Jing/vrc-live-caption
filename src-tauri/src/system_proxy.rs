//! Provider-neutral system-proxy route selection for cloud HTTPS connections.
//!
//! Environment and supported platform settings are read for every selection so
//! changing the system route does not require restarting the application.
//! Callers own connection mechanics and map the opaque failure into their own
//! diagnostic vocabulary.

use crate::error::{AppError, AppResult};
use hyper_util::client::proxy::matcher::Matcher;
use std::env::VarError;
use tungstenite::http::{HeaderValue, Uri};

/// The hostname and port that a selected HTTPS route must resolve and dial.
pub(crate) struct DialTarget {
    pub(crate) host: String,
    pub(crate) port: u16,
}

/// A complete fail-closed routing decision for one HTTPS target.
///
/// This intentionally has no `Debug` implementation because the proxy variant
/// can own a sensitive authorization header.
pub(crate) enum SelectedHttpsRoute {
    Direct {
        dial: DialTarget,
    },
    HttpConnect {
        dial: DialTarget,
        proxy_uri: Uri,
        proxy_authorization: Option<HeaderValue>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Provider-neutral rejection of route discovery or validation.
///
/// The shared seam deliberately retains none of the source error so provider
/// or platform details cannot escape through cloud-path diagnostics.
pub(crate) struct RouteSelectionFailure;

/// Selects the current environment or platform route for one HTTPS attempt.
pub(crate) fn select_https_route(
    target: &Uri,
) -> Result<SelectedHttpsRoute, RouteSelectionFailure> {
    select_https_route_with(target, || system_proxy_matcher(target))
}

fn select_https_route_with(
    target: &Uri,
    discover_matcher: impl FnOnce() -> AppResult<Matcher>,
) -> Result<SelectedHttpsRoute, RouteSelectionFailure> {
    if target.scheme_str() != Some("https") {
        return Err(RouteSelectionFailure);
    }
    let host = target.host().ok_or(RouteSelectionFailure)?;
    let direct = DialTarget {
        host: host.trim_matches(['[', ']']).to_string(),
        port: target.port_u16().unwrap_or(443),
    };
    let matcher = discover_matcher().map_err(|_| RouteSelectionFailure)?;
    let Some(proxy) = matcher.intercept(target) else {
        return Ok(SelectedHttpsRoute::Direct { dial: direct });
    };
    if proxy.uri().scheme_str() != Some("http") {
        return Err(RouteSelectionFailure);
    }
    let proxy_host = proxy.uri().host().ok_or(RouteSelectionFailure)?;
    Ok(SelectedHttpsRoute::HttpConnect {
        dial: DialTarget {
            host: proxy_host.trim_matches(['[', ']']).to_string(),
            port: proxy.uri().port_u16().unwrap_or(80),
        },
        proxy_uri: proxy.uri().clone(),
        proxy_authorization: proxy.basic_auth().cloned(),
    })
}

pub(crate) fn system_proxy_matcher(target: &Uri) -> AppResult<Matcher> {
    let https_proxy = first_environment_value(&["HTTPS_PROXY", "https_proxy"])?;
    let all_proxy = first_environment_value(&["ALL_PROXY", "all_proxy"])?;
    let explicit_proxy = https_proxy
        .filter(|value| !value.trim().is_empty())
        .or_else(|| all_proxy.filter(|value| !value.trim().is_empty()));

    matcher_for_proxy_sources(
        explicit_proxy,
        || first_environment_value(&["NO_PROXY", "no_proxy"]),
        || system_proxy_matcher_without_explicit_https(target),
    )
}

fn matcher_for_proxy_sources(
    explicit_proxy: Option<String>,
    no_proxy: impl FnOnce() -> AppResult<Option<String>>,
    system_matcher: impl FnOnce() -> AppResult<Matcher>,
) -> AppResult<Matcher> {
    if let Some(proxy) = explicit_proxy {
        let no_proxy = no_proxy()?;
        matcher_for_configured_https_proxy(&proxy, no_proxy.as_deref())
    } else {
        system_matcher()
    }
}

fn first_environment_value(names: &[&str]) -> AppResult<Option<String>> {
    for name in names {
        match std::env::var(name) {
            Ok(value) => return Ok(Some(value)),
            Err(VarError::NotPresent) => {}
            Err(VarError::NotUnicode(_)) => {
                return Err(AppError::recognition_network_terminal(format!(
                    "The {name} proxy setting is not valid Unicode; refusing a direct OpenAI connection."
                )));
            }
        }
    }
    Ok(None)
}

fn matcher_for_configured_https_proxy(proxy: &str, no_proxy: Option<&str>) -> AppResult<Matcher> {
    let proxy = normalized_http_proxy_uri(proxy)?;
    let mut builder = Matcher::builder().https(proxy);
    if let Some(no_proxy) = no_proxy {
        builder = builder.no(normalized_no_proxy(no_proxy));
    }
    Ok(builder.build())
}

fn normalized_http_proxy_uri(value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::recognition_network_terminal(
            "A proxy was selected for OpenAI Realtime but its address is empty.",
        ));
    }
    let candidate = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    let uri = candidate.parse::<Uri>().map_err(|error| {
        AppError::recognition_network_terminal(format!(
            "The configured OpenAI Realtime proxy address is invalid: {error}"
        ))
    })?;
    let scheme = uri.scheme_str().ok_or_else(|| {
        AppError::recognition_network_terminal(
            "The configured OpenAI Realtime proxy has no URI scheme.",
        )
    })?;
    if scheme != "http" {
        return Err(AppError::recognition_network_terminal(format!(
            "The selected system proxy scheme '{scheme}' is not supported for OpenAI Realtime; use an HTTP CONNECT proxy."
        )));
    }
    if uri.host().is_none() {
        return Err(AppError::recognition_network_terminal(
            "The configured OpenAI Realtime proxy has no host name.",
        ));
    }
    Ok(candidate)
}

fn normalized_no_proxy(value: &str) -> String {
    value
        .split(|character: char| matches!(character, ';' | ',') || character.is_ascii_whitespace())
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.eq_ignore_ascii_case("<local>"))
        .map(|entry| entry.strip_prefix("*.").unwrap_or(entry))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(any(target_os = "macos", test))]
mod macos;

#[cfg(any(target_os = "windows", test))]
mod windows;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn system_proxy_matcher_without_explicit_https(_target: &Uri) -> AppResult<Matcher> {
    // Linux has no additional supported system-proxy source. Reaching this branch
    // means no non-empty environment proxy was selected, so connect directly;
    // NO_PROXY alone does not select an environment route.
    Ok(direct_matcher(None))
}

#[cfg(target_os = "macos")]
fn system_proxy_matcher_without_explicit_https(target: &Uri) -> AppResult<Matcher> {
    macos::system_proxy_matcher(&target.to_string())
}

#[cfg(target_os = "windows")]
fn system_proxy_matcher_without_explicit_https(_target: &Uri) -> AppResult<Matcher> {
    windows::system_proxy_matcher(None)
}

fn direct_matcher(no_proxy: Option<&str>) -> Matcher {
    let mut builder = Matcher::builder();
    if let Some(no_proxy) = no_proxy {
        builder = builder.no(normalized_no_proxy(no_proxy));
    }
    builder.build()
}

#[cfg(any(target_os = "macos", test))]
fn authority_with_port(host: &str, port: u16) -> String {
    if host.starts_with('[') || !host.contains(':') {
        format!("{host}:{port}")
    } else {
        format!("[{host}]:{port}")
    }
}

#[cfg(test)]
#[path = "system_proxy_tests.rs"]
mod tests;
