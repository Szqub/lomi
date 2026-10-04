//! Source-authored gateway regressions. These fixtures use no real provider or
//! credential backend and must not be executed until implementation is complete.
use super::{gateway_config, gateway_profiles::Protocol, store::Store, types::*};
use crate::cli_catalog::TitleCli;

fn profile() -> Profile {
    Profile {
        id: "profile".into(),
        cli: TitleCli::Claude,
        label: "Fixture".into(),
        enabled: true,
        revision: 1,
        auth_state: AuthState::Ready,
        storage_mode: StorageMode::ApiKey,
        credential_ref: Some("fixture-reference".into()),
        quota_group_key: Some("fixture-binding".into()),
        gateway_provider: Some(ApiDestination {
            protocol: Protocol::Anthropic,
            base_url: "https://api.anthropic.com".into(),
        }),
    }
}
#[test]
fn destinations_admit_explicit_conversion_and_reject_stale_credential_bindings() {
    let provider = profile().gateway_provider.unwrap();
    let normalized = gateway_config::destination(TitleCli::Claude, &provider).unwrap();
    assert_eq!(normalized, provider);
    for base in [
        "http://api.anthropic.com",
        "https://user:password@api.anthropic.com",
        "https://api.anthropic.com?key=secret",
        "https://api.anthropic.com#secret",
        "https://api.anthropic.com/private/../v1",
    ] {
        assert!(gateway_config::destination(
            TitleCli::Claude,
            &ApiDestination {
                base_url: base.into(),
                ..provider.clone()
            }
        )
        .is_err());
    }
    assert!(gateway_config::destination(
        TitleCli::Claude,
        &ApiDestination {
            protocol: Protocol::OpenAiChat,
            ..provider
        }
    )
    .is_ok());
    let prefixed = ApiDestination {
        protocol: Protocol::Anthropic,
        base_url: "https://api.anthropic.com/private/".into(),
    };
    assert_eq!(
        gateway_config::destination(TitleCli::Claude, &prefixed)
            .unwrap()
            .base_url,
        "https://api.anthropic.com/private"
    );
    let mut account = profile();
    assert!(gateway_config::eligible(
        &account,
        TitleCli::Claude,
        Protocol::Anthropic
    ));
    assert!(gateway_config::eligible(
        &account,
        TitleCli::Claude,
        Protocol::OpenAiChat
    ));
    assert!(!gateway_config::eligible(
        &account,
        TitleCli::Codex,
        Protocol::Anthropic
    ));
    // An endpoint edit retains the old key reference for cleanup, but revokes
    // its destination binding even if stale UI state still says Ready.
    let next = ApiDestination {
        protocol: Protocol::Anthropic,
        base_url: "https://reviewed.example/v1".into(),
    };
    gateway_config::configure(&mut account, Some(&next)).unwrap();
    assert!(account.credential_ref.is_some());
    assert_eq!(account.auth_state, AuthState::Disconnected);
    assert!(account.quota_group_key.is_none());
    assert_eq!(account.revision, 2);
    account.auth_state = AuthState::Ready;
    assert!(!gateway_config::eligible(
        &account,
        TitleCli::Claude,
        Protocol::Anthropic
    ));
}
#[test]
fn gateway_pool_deduplicates_keys_by_effective_native_destination() {
    let state = super::CliRouterService::default();
    let mut snapshot = Snapshot::default();
    let mut first = profile();
    first.id = "a".into();
    let mut second = first.clone();
    second.id = "b".into();
    snapshot.profiles = vec![first, second];
    let router = Router {
        id: "router".into(),
        cli: TitleCli::Claude,
        label: "fixture".into(),
        enabled: true,
        ordered_profile_ids: vec!["a".into(), "b".into()],
        balance_remaining_quota: false,
        revision: 1,
    };
    for (first, second) in [
        ("https://example.com", "https://example.com/v1"),
        ("https://example.com/api", "https://example.com/api/v1"),
    ] {
        snapshot.profiles[0]
            .gateway_provider
            .as_mut()
            .unwrap()
            .base_url = first.into();
        snapshot.profiles[1]
            .gateway_provider
            .as_mut()
            .unwrap()
            .base_url = second.into();
        snapshot.profiles[1].quota_group_key = Some("fixture-binding".into());
        assert!(gateway_config::pool(&snapshot, &router, &state).is_err());
        snapshot.profiles[1].quota_group_key = Some("different-key".into());
        assert_eq!(
            gateway_config::pool(&snapshot, &router, &state)
                .unwrap()
                .len(),
            2
        );
    }
    snapshot.profiles[1].quota_group_key = Some("fixture-binding".into());
    for different in [
        "https://other.example/api/v1",
        "https://example.com/different/v1",
    ] {
        snapshot.profiles[1]
            .gateway_provider
            .as_mut()
            .unwrap()
            .base_url = different.into();
        assert_eq!(
            gateway_config::pool(&snapshot, &router, &state)
                .unwrap()
                .len(),
            2
        );
    }
    snapshot.profiles[1]
        .gateway_provider
        .as_mut()
        .unwrap()
        .protocol = Protocol::OpenAiChat;
    assert_eq!(
        gateway_config::pool(&snapshot, &router, &state)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn schema_four_profiles_migrate_to_five_without_inventing_gateway_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("router.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .update(|snapshot| {
            let mut old = profile();
            old.gateway_provider = None;
            snapshot.profiles.push(old);
            Ok(())
        })
        .unwrap();
    let before = store.snapshot().unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let old_bytes: String = connection
        .query_row("SELECT data FROM snapshot WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert!(!old_bytes.contains("gatewayProvider"));
    connection.pragma_update(None, "user_version", 4).unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot().unwrap(), before);
    assert!(store.snapshot().unwrap().profiles[0]
        .gateway_provider
        .is_none());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let migrated_bytes: String = connection
        .query_row("SELECT data FROM snapshot WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 5);
    assert_eq!(migrated_bytes, old_bytes);
}
