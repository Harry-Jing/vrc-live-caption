use super::*;

#[test]
fn selected_https_route_exposes_the_direct_origin_dial_target() -> Result<(), String> {
    let target = "https://translation.example.test:8443/v1/responses"
        .parse::<Uri>()
        .map_err(|error| format!("Failed to parse the test target: {error}"))?;

    let route = select_https_route_with(&target, || Ok(Matcher::builder().build()))
        .map_err(|_| "The direct HTTPS route was rejected.".to_string())?;

    match route {
        SelectedHttpsRoute::Direct { dial } => {
            assert_eq!(dial.host, "translation.example.test");
            assert_eq!(dial.port, 8443);
        }
        SelectedHttpsRoute::HttpConnect { .. } => {
            return Err("The direct HTTPS route unexpectedly selected a proxy.".to_string());
        }
    }
    Ok(())
}

#[test]
fn selected_https_route_exposes_the_proxy_dial_target_and_separated_auth() -> Result<(), String> {
    let target = "https://translation.example.test/v1/responses"
        .parse::<Uri>()
        .map_err(|error| format!("Failed to parse the test target: {error}"))?;
    let matcher = Matcher::builder()
        .https("http://proxy-user:proxy-password@proxy.example.test")
        .build();

    let route = select_https_route_with(&target, || Ok(matcher))
        .map_err(|_| "The HTTP CONNECT route was rejected.".to_string())?;

    match route {
        SelectedHttpsRoute::Direct { .. } => {
            return Err("The proxy HTTPS route unexpectedly selected Direct.".to_string());
        }
        SelectedHttpsRoute::HttpConnect {
            dial,
            proxy_uri,
            proxy_authorization,
        } => {
            assert_eq!(dial.host, "proxy.example.test");
            assert_eq!(dial.port, 80);
            assert_eq!(proxy_uri.to_string(), "http://proxy.example.test/");
            let authorization = proxy_authorization
                .ok_or_else(|| "The proxy authorization was not preserved.".to_string())?;
            assert_eq!(authorization, "Basic cHJveHktdXNlcjpwcm94eS1wYXNzd29yZA==");
            assert!(authorization.is_sensitive());
        }
    }
    Ok(())
}

#[test]
fn selected_https_route_rejects_non_https_with_an_opaque_failure() -> Result<(), String> {
    let target = "http://translation.example.test/v1/responses"
        .parse::<Uri>()
        .map_err(|error| format!("Failed to parse the test target: {error}"))?;

    let failure = select_https_route(&target)
        .err()
        .ok_or_else(|| "A non-HTTPS target unexpectedly selected a route.".to_string())?;

    assert_eq!(failure, RouteSelectionFailure);
    assert_eq!(format!("{failure:?}"), "RouteSelectionFailure");
    Ok(())
}

#[test]
fn selected_https_route_applies_the_existing_proxy_bypass_before_exposing_direct()
-> Result<(), String> {
    let target = "https://translation.example.test/v1/responses"
        .parse::<Uri>()
        .map_err(|error| format!("Failed to parse the test target: {error}"))?;
    let matcher = Matcher::builder()
        .https("http://proxy.example.test:8080")
        .no("translation.example.test")
        .build();

    let route = select_https_route_with(&target, || Ok(matcher))
        .map_err(|_| "The bypassed HTTPS route was rejected.".to_string())?;

    match route {
        SelectedHttpsRoute::Direct { dial } => {
            assert_eq!(dial.host, "translation.example.test");
            assert_eq!(dial.port, 443);
        }
        SelectedHttpsRoute::HttpConnect { .. } => {
            return Err("The bypassed HTTPS route still exposed a proxy.".to_string());
        }
    }
    Ok(())
}

#[test]
fn selected_https_route_discards_discovery_errors() -> Result<(), String> {
    let target = "https://translation.example.test/v1/responses"
        .parse::<Uri>()
        .map_err(|error| format!("Failed to parse the test target: {error}"))?;

    let failure = select_https_route_with(&target, || {
        Err(AppError::recognition_network_terminal(
            "private-route-detail",
        ))
    })
    .err()
    .ok_or_else(|| "Failed route discovery unexpectedly selected Direct.".to_string())?;

    assert_eq!(std::mem::size_of_val(&failure), 0);
    assert_eq!(format!("{failure:?}"), "RouteSelectionFailure");
    Ok(())
}

#[test]
fn selected_https_route_rejects_an_unsupported_proxy_without_direct_fallback() -> Result<(), String>
{
    let target = "https://translation.example.test/v1/responses"
        .parse::<Uri>()
        .map_err(|error| format!("Failed to parse the test target: {error}"))?;
    let matcher = Matcher::builder()
        .https("socks5://proxy.example.test:1080")
        .build();

    let failure = select_https_route_with(&target, || Ok(matcher))
        .err()
        .ok_or_else(|| "The unsupported proxy unexpectedly selected a route.".to_string())?;

    assert_eq!(failure, RouteSelectionFailure);
    Ok(())
}

#[test]
fn malformed_configured_proxy_is_invalid_instead_of_direct() -> AppResult<()> {
    let failure = matcher_for_configured_https_proxy("http://[invalid", None)
        .err()
        .ok_or_else(|| AppError::state("A malformed configured proxy was treated as direct."))?;

    assert_eq!(failure.code(), "stt.network_unreachable");
    assert!(failure.to_string().contains("proxy address is invalid"));
    Ok(())
}

#[test]
fn proxy_bypass_accepts_windows_whitespace_and_environment_commas() {
    assert_eq!(
        normalized_no_proxy(
            "*.internal.example localhost\tservice.example,environment.example;last.example"
        ),
        "internal.example,localhost,service.example,environment.example,last.example"
    );
}

#[test]
fn unpaired_environment_no_proxy_does_not_override_the_system_route() -> AppResult<()> {
    let no_proxy_was_read = std::cell::Cell::new(false);
    let matcher = matcher_for_proxy_sources(
        None,
        || {
            no_proxy_was_read.set(true);
            Ok(Some("api.openai.com".to_string()))
        },
        || {
            Ok(Matcher::builder()
                .https("http://system-proxy.example:8443")
                .build())
        },
    )?;
    let target = "https://api.openai.com/"
        .parse::<Uri>()
        .map_err(|error| AppError::state(format!("Failed to parse test URI: {error}")))?;
    let selected = matcher.intercept(&target).ok_or_else(|| {
        AppError::state("An unpaired environment NO_PROXY overrode the selected system route.")
    })?;

    assert_eq!(selected.uri().host(), Some("system-proxy.example"));
    assert_eq!(selected.uri().port_u16(), Some(8443));
    assert!(!no_proxy_was_read.get());
    Ok(())
}

#[test]
fn explicit_environment_proxy_pairs_with_no_proxy_and_precedes_the_system_route() -> AppResult<()> {
    let system_route_was_read = std::cell::Cell::new(false);
    let matcher = matcher_for_proxy_sources(
        Some("http://environment-proxy.example:8080".to_string()),
        || Ok(Some("api.openai.com".to_string())),
        || {
            system_route_was_read.set(true);
            Ok(Matcher::builder()
                .https("http://system-proxy.example:8443")
                .build())
        },
    )?;
    let target = "https://api.openai.com/"
        .parse::<Uri>()
        .map_err(|error| AppError::state(format!("Failed to parse test URI: {error}")))?;

    assert!(matcher.intercept(&target).is_none());
    assert!(!system_route_was_read.get());
    Ok(())
}
