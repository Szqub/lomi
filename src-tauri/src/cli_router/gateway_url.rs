//! Canonical, explicitly configured HTTPS API prefixes and native route joining.
use super::gateway_profiles::Protocol;
use reqwest::Url;

fn failure() -> String {
    "Use an absolute HTTPS provider API base without credentials, query, fragment or ambiguous path segments.".into()
}

pub(crate) fn parse(base: &str) -> Result<Url, String> {
    if base.is_empty()
        || base.len() > 2048
        || base.chars().any(|c| c.is_control() || c.is_whitespace())
        || base.contains('\\')
    {
        return Err(failure());
    }
    // Inspect the original path before Url can normalize dot segments.
    let (_, authority_and_path) = base.split_once("://").ok_or_else(failure)?;
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .ok_or_else(failure)?;
    if authority.contains('@') {
        return Err(failure());
    }
    let original_path = authority_and_path
        .find('/')
        .map_or("", |index| &authority_and_path[index..]);
    let path = original_path.strip_suffix('/').unwrap_or(original_path);
    if !path.is_empty()
        && path.strip_prefix('/').is_none_or(|rest| {
            rest.split('/').any(|part| {
                part.is_empty()
                    || matches!(part, "." | "..")
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.~".contains(&b))
            })
        })
    {
        return Err(failure());
    }
    let url = Url::parse(base).map_err(|_| failure())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(failure());
    }
    Ok(url)
}

pub(crate) fn canonical(base: &str) -> Result<String, String> {
    Ok(parse(base)?.as_str().trim_end_matches('/').to_owned())
}

/// Only the official origin/version grants documented signature portability.
pub(crate) fn official_anthropic(base: &str) -> bool {
    parse(base).is_ok_and(|url| {
        url.host_str() == Some("api.anthropic.com")
            && url.port_or_known_default() == Some(443)
            && matches!(url.path().trim_end_matches('/'), "" | "/v1")
    })
}

/// Effective native destinations for credential-pool deduplication. The
/// Gemini pair preserves both supported versions for an unversioned base.
pub(crate) fn dispatch_identity(base: &str, protocol: Protocol) -> Result<Vec<String>, String> {
    let routes: &[&str] = match protocol {
        Protocol::Anthropic => &["/v1/messages"],
        Protocol::OpenAiChat => &["/v1/chat/completions"],
        Protocol::OpenAiResponses => &["/v1/responses"],
        Protocol::Gemini => &[
            "/v1/models/lomi-destination-identity:generateContent",
            "/v1beta/models/lomi-destination-identity:generateContent",
        ],
    };
    let mut identity = routes
        .iter()
        .map(|route| join(base, protocol, route).map(|url| url.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    identity.sort();
    identity.dedup();
    Ok(identity)
}

/// A route has already passed the gateway's selected-model/body admission.
/// Recheck its native protocol shape here before attaching any API prefix.
pub(crate) fn join(base: &str, protocol: Protocol, route: &str) -> Result<Url, String> {
    let canonical = canonical(base)?;
    let (path, query) = route
        .split_once('?')
        .map_or((route, None), |(p, q)| (p, Some(q)));
    let native = match protocol {
        Protocol::Anthropic
            if matches!(path, "/v1/messages" | "/v1/messages/count_tokens") && query.is_none() =>
        {
            path.strip_prefix("/v1").map(|tail| ("/v1", tail))
        }
        Protocol::OpenAiChat if path == "/v1/chat/completions" && query.is_none() => {
            Some(("/v1", "/chat/completions"))
        }
        Protocol::OpenAiResponses if path == "/v1/responses" && query.is_none() => {
            Some(("/v1", "/responses"))
        }
        Protocol::Gemini => ["/v1beta", "/v1"].into_iter().find_map(|version| {
            let tail = path.strip_prefix(version)?.strip_prefix("/models/")?;
            let (model, stream) = if let Some(model) = tail.strip_suffix(":streamGenerateContent") {
                (model, true)
            } else {
                (
                    tail.strip_suffix(":generateContent")
                        .or_else(|| tail.strip_suffix(":countTokens"))?,
                    false,
                )
            };
            if model.is_empty()
                || model.len() > 256
                || !model
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
                || model
                    .split('/')
                    .any(|part| part.is_empty() || matches!(part, "." | ".."))
                || if stream {
                    query != Some("alt=sse")
                } else {
                    query.is_some()
                }
            {
                return None;
            }
            Some((version, path.strip_prefix(version)?))
        }),
        _ => None,
    }
    .ok_or_else(failure)?;
    let mut url = Url::parse(&canonical).map_err(|_| failure())?;
    let prefix = url.path().trim_end_matches('/');
    let versioned = prefix.ends_with("/v1") || prefix.ends_with("/v1beta");
    let target = if versioned {
        format!("{prefix}{}", native.1)
    } else {
        format!("{prefix}{}{}", native.0, native.1)
    };
    url.set_path(&target);
    url.set_query(query);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_routes_preserve_explicit_provider_prefixes_and_versions() {
        for (base, protocol, route, expected) in [
            ("https://openrouter.ai/api/v1/", Protocol::OpenAiChat, "/v1/chat/completions", "https://openrouter.ai/api/v1/chat/completions"),
            ("https://example.com/compatible-mode/v1", Protocol::OpenAiResponses, "/v1/responses", "https://example.com/compatible-mode/v1/responses"),
            ("https://example.com/anthropic", Protocol::Anthropic, "/v1/messages", "https://example.com/anthropic/v1/messages"),
            ("https://example.com/anthropic/v1", Protocol::Anthropic, "/v1/messages/count_tokens", "https://example.com/anthropic/v1/messages/count_tokens"),
            ("https://example.com/gemini/v1beta", Protocol::Gemini, "/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse", "https://example.com/gemini/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"),
            ("https://example.com/gemini", Protocol::Gemini, "/v1/models/gemini-2.5-pro:generateContent", "https://example.com/gemini/v1/models/gemini-2.5-pro:generateContent"),
        ] {
            assert_eq!(join(base, protocol, route).unwrap().as_str(), expected);
            assert_eq!(canonical(&canonical(base).unwrap()).unwrap(), canonical(base).unwrap());
        }
    }
    #[test]
    fn canonical_hosts_ports_and_prefix_trailing_slashes_bind_the_same_destination() {
        assert_eq!(
            canonical("HTTPS://OPENROUTER.AI:443/api/v1/").unwrap(),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            join(
                "HTTPS://OPENROUTER.AI:443/api/v1/",
                Protocol::OpenAiChat,
                "/v1/chat/completions"
            )
            .unwrap()
            .as_str(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert!(canonical("https://@example.com/api/v1").is_err());
    }
    #[test]
    fn custom_prefixes_do_not_inherit_official_anthropic_portability() {
        for base in [
            "https://api.anthropic.com",
            "https://api.anthropic.com/",
            "https://api.anthropic.com:443/v1/",
        ] {
            assert!(official_anthropic(base));
        }
        for base in [
            "https://api.anthropic.com/proxy/v1",
            "https://api.anthropic.com/v1beta",
            "https://api.anthropic.com:8443/v1",
            "https://api.anthropic.com.example/v1",
            "http://api.anthropic.com/v1",
            "https://api.anthropic.com/v1?key=x",
        ] {
            assert!(!official_anthropic(base));
        }
    }
    #[test]
    fn dispatch_identity_collapses_only_equivalent_native_destinations() {
        for protocol in [
            Protocol::Anthropic,
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
        ] {
            assert_eq!(
                dispatch_identity("https://example.com", protocol).unwrap(),
                dispatch_identity("https://example.com/v1", protocol).unwrap()
            );
            assert_eq!(
                dispatch_identity("https://example.com/api", protocol).unwrap(),
                dispatch_identity("https://example.com/api/v1", protocol).unwrap()
            );
            assert_ne!(
                dispatch_identity("https://example.com/api", protocol).unwrap(),
                dispatch_identity("https://example.com/other", protocol).unwrap()
            );
        }
        let native = dispatch_identity("https://example.com/api", Protocol::Gemini).unwrap();
        assert_eq!(native.len(), 2);
        assert_ne!(
            native,
            dispatch_identity("https://example.com/api/v1", Protocol::Gemini).unwrap()
        );
        assert_ne!(
            native,
            dispatch_identity("https://example.com/api/v1beta", Protocol::Gemini).unwrap()
        );
        assert_ne!(
            dispatch_identity("https://example.com/api/v1", Protocol::Gemini).unwrap(),
            dispatch_identity("https://example.com/api/v1beta", Protocol::Gemini).unwrap()
        );
    }
    #[test]
    fn ambiguous_destinations_and_non_native_queries_are_rejected() {
        for base in [
            "http://example.com/api/v1",
            "https://user:secret@example.com",
            "https://example.com?key=x",
            "https://example.com#x",
            "https://example.com/a/../v1",
            "https://example.com/a/%2e%2e/v1",
            "https://example.com/a//v1",
            "https://example.com/a\\v1",
            "https://example.com/\nv1",
            "https://example.com/v1//",
        ] {
            assert!(canonical(base).is_err(), "{base:?}");
        }
        for (protocol, route) in [
            (Protocol::Anthropic, "/v1/messages?alt=sse"),
            (Protocol::OpenAiResponses, "/v1/responses?key=x"),
            (
                Protocol::Gemini,
                "/v1beta/models/gemini:generateContent?alt=sse",
            ),
            (
                Protocol::Gemini,
                "/v1beta/models/gemini:streamGenerateContent?alt=sse&key=x",
            ),
            (Protocol::Gemini, "/v1beta/models/../gemini:generateContent"),
        ] {
            assert!(join("https://example.com/api/v1", protocol, route).is_err());
        }
    }
}
