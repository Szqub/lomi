# S01 native/WASM crypto prototype — unqualified

`PRODUCTION_QUALIFIED` and the browser `production_qualified()` are always false.
This independent crate is not wired into any application, host, or network path.
Passing its tests does not accept S01 or authorize production remote access.

Pinned suites and libraries:

- Snow 0.10.0, `Noise_XX_25519_ChaChaPoly_SHA256`, Rust default resolver.
- HPKE 0.14.1, X25519/HKDF-SHA256/ChaCha20Poly1305 base mode, with an additional
  strict Ed25519 signature covering the complete envelope.
- ed25519-dalek 2.2.0; SHA-256 fingerprints with sha2 0.10.9.
- Canonical CBOR encoding with minicbor 0.26.5; wasm-bindgen 0.2.126.
- getrandom 0.3.4 `wasm_js` for Snow and 0.4.3 `wasm_js` for HPKE/application
  entropy on WASM. Native uses platform entropy. Snow `std` is enabled only on
  native because it unnecessarily pulls ring into a WASM build; the suite and
  default resolver are identical on both targets.

The native typed API and browser WASM API share the exact implementation. Browser
identities are ephemeral: there is no private-key export/import, wrapping key,
IndexedDB persistence, or trusted-PWA mode. `test-fixtures` exposes deterministic
test identities only when explicitly built with that feature. Never ship those
fixture builds. Entropy comes from maintained libraries/WebCrypto; no custom
cryptographic primitive or random-number generator is implemented.

## Trust and context binding

A signed public bundle binds schema version, account ID, subject ID, host/device
role, key version, Ed25519 identity key, channel X25519 key, and distinct mailbox
HPKE public key. Its SHA-256 fingerprint covers the canonical complete public
bundle. Callers must supply a fingerprint already trusted through pairing.
Self-signature validation alone does not grant trust; there is no silent TOFU.

Noise prologue binds protocol version, account, host, device, initiator/responder
roles, and both full-bundle fingerprints. The only supported role ordering is
Device initiator → Host responder. Remote static keys are checked against the
verified pinned bundle before transport is constructed. No application payload
is permitted during handshake. Channel handles cannot be cloned, and mutable
ownership serializes all nonce operations. Each encrypted transport payload
binds its external sequence and routing identifier. Authentication, ordering,
or routing failure closes the channel; reconnect needs a fresh handshake.

Mailbox HPKE info binds canonical metadata and sender/recipient full-bundle
fingerprints. Metadata is also AEAD AAD. The Ed25519 signature covers canonical
metadata, HPKE `enc`, and ciphertext. Current trusted policy supplied by the
caller must match account, host, recipient, grant_ref, access_epoch, revision,
and grant deadline. Recipient key version, creation time, expiry, maximum
seven-day retention, sender role, recipient role, and signatures are checked
before decryption. Envelope IDs are freshly random 128-bit identifiers. Retry
must reuse the original serialized envelope through a future durable outbox.

Replay state is bounded to 5000 unexpired `(sender_host_id,envelope_id)` entries
and refuses admission at capacity. Expired entries may be removed because the
same envelope is then rejected by expiry. This state is in memory only; it is
not a durable replay store. Replay-cache pruning and its clock watermark commit only after successful
authenticated decryption. A successfully observed clock cannot move backwards.
Policy validation and trusted time still belong to the authoritative host/client
policy layer; this crate does not establish grant authority by itself.

## Provisional canonical encoding

Signed/hashed data is a definite-length CBOR array, with shortest integer/length
encodings; IDs and keys are byte strings. Every array starts with the text strings
`lomi-s01-unqualified-v1` and its domain below. Array fields are positional:

| Domain | Remaining fields, in order |
| --- | --- |
| `public-bundle` | version, account_id, subject_id, role (host=1/device=2), key_version, identity_key, channel_key, mailbox_key |
| `channel-context` | version, account_id, host_id, device_id, initiator_role, responder_role |
| `noise-prologue` | canonical context bytes, device fingerprint, host fingerprint |
| `mailbox-metadata` | version, account_id, sender_host_id, recipient_device_id, recipient_key_version, envelope_id, grant_ref, access_epoch, grant_revision, created_at, expires_at |
| `hpke-info` | canonical metadata bytes, sender fingerprint, recipient fingerprint |
| `signed-envelope` | canonical metadata bytes, enc, ciphertext |

JSON is an API adapter representation, never the signature encoding. Browser
callers should retain serialized JSON without number conversions; u64 values
outside JS's exact integer range must not be parsed and reconstructed through
ordinary JS numbers. Binary framing and durable schema compatibility are still
future work.

## Limits and unfinished security gates

Handshake frames: 512 bytes. Live plaintext: 16 KiB; encrypted header: 24 bytes;
AEAD tag: 16 bytes. Per-direction channel lifetime: 1,000,000 frames, followed
by a required new handshake. Mailbox plaintext: 8 KiB; `enc`: 32 bytes; signature:
64 bytes. Browser JSON inputs/replies: 128 KiB. Typed native APIs check vector
sizes before cryptographic processing. wasm-bindgen copies JS inputs before Rust
validation, so an application adapter must also bound values before crossing the
WASM boundary. No decompression is implemented.

**Zeroization is a blocking S01 qualification failure.** Stock Snow's resolver
contains raw key arrays and symmetric temporaries without proven zeroizing Drop.
Application `Zeroizing` wrappers do not erase Snow's copies, Rust temporaries,
allocator remnants, WASM linear-memory copies, browser JS buffers, or snapshots.
No forward-secrecy or complete secret-erasure claim is made by this prototype.
HPKE's convenience sender uses upstream system randomness and may panic if its
entropy source fails; failure-injection behavior has not been qualified. Native
identity generation reports entropy errors explicitly. Key-store integration,
rollback/revoke durability, pairing transcript approval, cryptographic fuzzing,
external reference-vector validation, independent review, browser persistence,
Safari/Firefox/mobile qualification, and deployment controls remain unfinished.

## Reproduce validation

```sh
cargo test --manifest-path Cargo.toml --locked
cargo test --manifest-path Cargo.toml --locked --features browser,test-fixtures
cargo clippy --manifest-path Cargo.toml --locked --all-targets --features browser,test-fixtures -- -D warnings
cargo build --manifest-path Cargo.toml --locked --target wasm32-unknown-unknown --features browser,test-fixtures
wasm-bindgen target/wasm32-unknown-unknown/debug/lomi_remote_crypto.wasm --out-dir pkg --target web
node tests/browser.mjs
```

The browser runner requires the enclosing Lomi project's Playwright dependency
and installed Chromium. It starts a temporary loopback HTTP server for test
artifacts only. Actual Chromium tests exercise entropy, Noise/Unicode transport,
negative pin/tamper/expiry/replay cases, matching native/WASM fixed bundle
fingerprints, native-to-WASM HPKE decryption, and WASM-to-native HPKE decryption.
Native/WASM mixed-process Noise interoperability and other browser engines have
not been tested. Generated `pkg/` and `target/` are ignored. This crate has its
own workspace marker and lockfile until an explicit parent-workspace integration.

## Isolated prototype result and integration work

Verified on macOS arm64 with rustc 1.98.1: nine default native tests and ten
native tests with browser/fixture features pass. Strict Clippy and formatting
checks pass. The real WASM build and Chromium runner pass eleven checks,
including bidirectional native/WASM HPKE decryption and fixed signed-bundle
fingerprints. These are prototype compatibility results; S01 remains unqualified.

Before integration, remove the independent workspace marker and reconcile pinned
features/lockfiles in the parent workspace. Close the documented zeroization,
entropy-failure, replay durability, pairing, external-vector, fuzzing, and review
gates; bind trusted policy/clock and pinned bundles to the authoritative host.
Add a bounded binary network adapter and browser boundary checks, qualified
native key storage and optional browser wrapping/persistence, and a release
process that excludes test-fixture builds. No application runtime should depend
on this crate for production remote access while qualification remains false.
