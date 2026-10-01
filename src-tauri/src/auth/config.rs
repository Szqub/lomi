use reqwest::Url;

const DEFAULT_ORIGIN: &str = "https://auth.lomi.dev";
const DEFAULT_CLIENT_ID: &str = "lomi-desktop";

#[derive(Clone)]
pub(super) struct AuthConfig {
    pub origin: String,
    pub client_id: String,
    pub environment: String,
}

impl AuthConfig {
    pub fn from_build() -> Result<Self, String> {
        Self::for_build(
            cfg!(debug_assertions),
            option_env!("LOMI_AUTH_ORIGIN"),
            option_env!("LOMI_AUTH_CLIENT_ID"),
        )
    }

    fn for_build(
        is_debug: bool,
        origin_override: Option<&str>,
        client_id_override: Option<&str>,
    ) -> Result<Self, String> {
        let (origin, client_id) = if is_debug {
            (
                origin_override.unwrap_or(DEFAULT_ORIGIN),
                client_id_override.unwrap_or(DEFAULT_CLIENT_ID),
            )
        } else {
            (DEFAULT_ORIGIN, DEFAULT_CLIENT_ID)
        };
        Self::new(origin, client_id, is_debug)
    }

    fn new(origin: &str, client_id: &str, allow_local_http: bool) -> Result<Self, String> {
        if client_id.is_empty()
            || client_id.len() > 100
            || !client_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            return Err("The account sign-in configuration is invalid.".into());
        }
        let parsed = parse_origin(origin, allow_local_http)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "The account sign-in configuration is invalid.".to_string())?;
        let environment = if is_loopback(host) {
            "development".to_string()
        } else {
            host.to_ascii_lowercase().replace('.', "-")
        };
        let mut canonical = parsed.to_string();
        if canonical.ends_with('/') {
            canonical.pop();
        }
        Ok(Self {
            origin: canonical,
            client_id: client_id.to_string(),
            environment,
        })
    }

    pub fn endpoint(&self, path: &str) -> Result<Url, String> {
        Url::parse(&format!("{}{}", self.origin, path))
            .map_err(|_| "The account service address is invalid.".to_string())
    }

    pub fn remote_origin(&self) -> Result<String, String> {
        let auth = Url::parse(&self.origin).map_err(|_| "Invalid account origin.")?;
        let expected = match auth.host_str() {
            Some("auth.lomi.dev") => "https://remote.lomi.dev",
            Some("auth-staging.lomi.dev") => "https://remote-staging.lomi.dev",
            Some(host) if cfg!(debug_assertions) && is_loopback(host) => "http://127.0.0.1:3002",
            _ => return Err("Remote is not configured for this account environment.".into()),
        };
        let configured = if cfg!(debug_assertions) {
            option_env!("LOMI_REMOTE_ORIGIN").unwrap_or(expected)
        } else {
            expected
        };
        let parsed = Url::parse(configured).map_err(|_| "Invalid Remote origin.")?;
        if parsed.as_str().trim_end_matches('/') != expected
            && !(cfg!(debug_assertions)
                && expected == "http://127.0.0.1:3002"
                && parsed.as_str().trim_end_matches('/') == "http://127.0.0.1:4322")
        {
            return Err("Remote origin does not match the account environment.".into());
        }
        Ok(parsed.as_str().trim_end_matches('/').into())
    }

    pub fn desktop_authorization_url(&self, raw: &str) -> Result<String, String> {
        let parsed = Url::parse(raw)
            .map_err(|_| "The account service returned an invalid sign-in address.".to_string())?;
        let base = Url::parse(&self.origin)
            .map_err(|_| "The account service address is invalid.".to_string())?;
        if parsed.origin() != base.origin()
            || parsed.path() != "/desktop"
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("The account service returned an untrusted sign-in address.".into());
        }
        let request = parsed
            .query()
            .and_then(|query| query.strip_prefix("request="))
            .filter(|request| valid_request_id(request))
            .ok_or_else(|| {
                "The account service returned an untrusted sign-in address.".to_string()
            })?;
        if parsed.as_str() != format!("{}/desktop?request={request}", self.origin) {
            return Err("The account service returned an untrusted sign-in address.".into());
        }
        Ok(parsed.to_string())
    }

    pub fn account_portal_uri(&self) -> Result<String, String> {
        self.endpoint("/account").map(|url| url.to_string())
    }
}

fn valid_request_id(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn parse_origin(origin: &str, allow_local_http: bool) -> Result<Url, String> {
    let parsed = Url::parse(origin)
        .map_err(|_| "The account sign-in configuration is invalid.".to_string())?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "The account sign-in configuration is invalid.".to_string())?;
    let local = is_loopback(host);
    let secure_owned_host = host == "lomi.dev" || host.ends_with(".lomi.dev");
    let valid_scheme = if parsed.scheme() == "https" {
        parsed.port().is_none_or(|port| port == 443) && secure_owned_host
    } else {
        allow_local_http
            && parsed.scheme() == "http"
            && local
            && (parsed.port_or_known_default() == Some(4321)
                || cfg!(feature = "remote-probe") && parsed.port_or_known_default() == Some(4324))
    };
    if !valid_scheme
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("The account sign-in origin is not trusted.".into());
    }
    Ok(parsed)
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_confirmed_release_origin() {
        let config = AuthConfig::new(DEFAULT_ORIGIN, DEFAULT_CLIENT_ID, false).unwrap();
        assert_eq!(config.origin, "https://auth.lomi.dev");
        assert_eq!(config.environment, "auth-lomi-dev");
    }

    #[test]
    fn debug_build_defaults_to_production_but_allows_explicit_overrides() {
        let defaults = AuthConfig::for_build(true, None, None).unwrap();
        assert_eq!(defaults.origin, DEFAULT_ORIGIN);
        assert_eq!(defaults.client_id, DEFAULT_CLIENT_ID);

        let local = AuthConfig::for_build(
            true,
            Some("http://localhost:4321"),
            Some("lomi-desktop-dev"),
        )
        .unwrap();
        assert_eq!(local.origin, "http://localhost:4321");
        assert_eq!(local.client_id, "lomi-desktop-dev");
        assert_eq!(local.environment, "development");
    }

    #[test]
    fn release_build_ignores_auth_overrides_and_uses_production() {
        let config = AuthConfig::for_build(
            false,
            Some("http://localhost:4321"),
            Some("lomi-desktop-dev"),
        )
        .unwrap();
        assert_eq!(config.origin, DEFAULT_ORIGIN);
        assert_eq!(config.client_id, DEFAULT_CLIENT_ID);
        assert_eq!(config.environment, "auth-lomi-dev");
    }

    #[test]
    fn rejects_lookalike_and_untrusted_origins() {
        for origin in [
            "https://auth.lomi.dev.attacker.test",
            "https://evil.test",
            "https://auth.lomi.dev:444",
            "https://user@auth.lomi.dev",
            "https://auth.lomi.dev/api/auth",
            "https://auth.lomi.dev?next=https://evil.test",
        ] {
            assert!(
                AuthConfig::new(origin, "lomi-desktop", false).is_err(),
                "{origin}"
            );
        }
    }

    #[test]
    fn local_http_is_debug_only_and_pinned_to_the_dev_port() {
        assert!(AuthConfig::new("http://localhost:4321", "lomi-desktop-dev", true).is_ok());
        assert!(AuthConfig::new("http://localhost:4322", "lomi-desktop-dev", true).is_err());
        assert!(AuthConfig::new("http://example.test:4321", "lomi-desktop-dev", true).is_err());
        assert!(AuthConfig::new("http://localhost:4321", "lomi-desktop-dev", false).is_err());
    }

    #[test]
    fn desktop_authorization_url_must_be_the_fixed_request_page() {
        let config = AuthConfig::new("https://auth.lomi.dev", "lomi-desktop", false).unwrap();
        assert_eq!(
            config
                .desktop_authorization_url(&format!(
                    "https://auth.lomi.dev/desktop?request={}",
                    "A".repeat(43)
                ))
                .unwrap(),
            format!("https://auth.lomi.dev/desktop?request={}", "A".repeat(43))
        );
        for uri in [
            "https://evil.test/desktop?request=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "https://auth.lomi.dev/desktop/other?request=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "https://auth.lomi.dev/desktop?request=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&extra=1",
            "https://auth.lomi.dev/desktop?request=short",
            "https://auth.lomi.dev/account?request=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "https://auth.lomi.dev/desktop?request=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA#fragment",
        ] {
            assert!(config.desktop_authorization_url(uri).is_err(), "{uri}");
        }
    }

    #[test]
    fn invalid_client_ids_are_rejected() {
        assert!(AuthConfig::new("https://auth.lomi.dev", "lomi desktop", false).is_err());
        assert!(AuthConfig::new("https://auth.lomi.dev", "", false).is_err());
    }
}
