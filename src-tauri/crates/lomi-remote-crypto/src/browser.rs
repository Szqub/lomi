use crate::{
    Channel, ChannelContext, Envelope, Handshake, Identity, MailboxPolicy, Role, SignedBundle,
    TransportFrame, MAX_JSON,
};
use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::prelude::*;
fn error(message: impl AsRef<str>) -> JsValue {
    JsValue::from_str(message.as_ref())
}
fn decode<T: DeserializeOwned>(json: &str) -> Result<T, JsValue> {
    if json.len() > MAX_JSON {
        return Err(error("JSON size limit"));
    }
    serde_json::from_str(json).map_err(|_| error("invalid JSON input"))
}
fn encode<T: Serialize>(value: &T) -> Result<String, JsValue> {
    let json = serde_json::to_string(value).map_err(|_| error("JSON encoding failed"))?;
    if json.len() > MAX_JSON {
        return Err(error("JSON reply limit"));
    }
    Ok(json)
}
fn fixed<const N: usize>(bytes: &[u8]) -> Result<[u8; N], JsValue> {
    bytes
        .try_into()
        .map_err(|_| error("wrong byte array length"))
}
fn role(host: bool) -> Role {
    if host {
        Role::Host
    } else {
        Role::Device
    }
}
/// Ephemeral identity only. JS plaintext copies are outside Rust zeroization.
#[wasm_bindgen]
pub struct BrowserIdentity(Identity);
#[wasm_bindgen]
impl BrowserIdentity {
    #[wasm_bindgen(constructor)]
    pub fn new(
        account_id: &[u8],
        subject_id: &[u8],
        host: bool,
        key_version: u32,
    ) -> Result<BrowserIdentity, JsValue> {
        Identity::generate(
            fixed(account_id)?,
            fixed(subject_id)?,
            role(host),
            key_version,
        )
        .map(Self)
        .map_err(error)
    }
    pub fn dispose(&mut self) {
        self.0.dispose();
    }
    pub fn public_bundle(&self) -> Result<String, JsValue> {
        encode(&self.0.public_bundle())
    }
    pub fn fingerprint(&self) -> Result<Vec<u8>, JsValue> {
        self.0
            .public_bundle()
            .bundle
            .fingerprint()
            .map(|p| p.to_vec())
            .map_err(error)
    }
    pub fn seal_mailbox(
        &self,
        recipient_json: &str,
        pinned_recipient: &[u8],
        policy_json: &str,
        now: u64,
        expires_at: u64,
        payload: &[u8],
    ) -> Result<String, JsValue> {
        let recipient: SignedBundle = decode(recipient_json)?;
        let policy: MailboxPolicy = decode(policy_json)?;
        encode(
            &self
                .0
                .seal_mailbox(
                    &recipient,
                    &fixed(pinned_recipient)?,
                    &policy,
                    now,
                    expires_at,
                    payload,
                )
                .map_err(error)?,
        )
    }
    pub fn open_mailbox(
        &mut self,
        envelope_json: &str,
        sender_json: &str,
        pinned_sender: &[u8],
        policy_json: &str,
        now: u64,
    ) -> Result<Vec<u8>, JsValue> {
        let envelope: Envelope = decode(envelope_json)?;
        let sender: SignedBundle = decode(sender_json)?;
        let policy: MailboxPolicy = decode(policy_json)?;
        self.0
            .open_mailbox(&envelope, &sender, &fixed(pinned_sender)?, &policy, now)
            .map(|p| p.to_vec())
            .map_err(error)
    }
    #[cfg(feature = "test-fixtures")]
    pub fn fixture(host: bool, seed: u8) -> Result<BrowserIdentity, JsValue> {
        Identity::fixture(role(host), seed).map(Self).map_err(error)
    }
}
#[wasm_bindgen]
pub struct BrowserHandshake(Handshake);
#[wasm_bindgen]
impl BrowserHandshake {
    #[wasm_bindgen(constructor)]
    pub fn new(
        initiator: bool,
        identity: &BrowserIdentity,
        context_json: &str,
        peer_json: &str,
        pinned_peer: &[u8],
    ) -> Result<BrowserHandshake, JsValue> {
        let context: ChannelContext = decode(context_json)?;
        let peer: SignedBundle = decode(peer_json)?;
        Handshake::new(
            initiator,
            &identity.0,
            &context,
            &peer,
            &fixed(pinned_peer)?,
        )
        .map(Self)
        .map_err(error)
    }
    pub fn dispose(&mut self) {
        self.0.dispose();
    }
    pub fn write(&mut self) -> Result<Vec<u8>, JsValue> {
        self.0.write().map_err(error)
    }
    pub fn read(&mut self, message: &[u8]) -> Result<(), JsValue> {
        self.0.read(message).map_err(error)
    }
    pub fn finish(self) -> Result<BrowserChannel, JsValue> {
        self.0.finish().map(BrowserChannel).map_err(error)
    }
}
#[wasm_bindgen]
pub struct BrowserChannel(Channel);
#[wasm_bindgen]
impl BrowserChannel {
    pub fn dispose(&mut self) {
        self.0.dispose();
    }
    pub fn seal_binary(&mut self, routing: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.0
            .seal_binary(fixed(routing)?, plaintext)
            .map_err(error)
    }
    pub fn open_binary(&mut self, routing: &[u8], frame: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.0
            .open_binary(fixed(routing)?, frame)
            .map(|p| p.to_vec())
            .map_err(error)
    }
    pub fn seal(&mut self, routing: &[u8], plaintext: &[u8]) -> Result<String, JsValue> {
        encode(&self.0.seal(fixed(routing)?, plaintext).map_err(error)?)
    }
    pub fn open(&mut self, routing: &[u8], frame_json: &str) -> Result<Vec<u8>, JsValue> {
        let frame: TransportFrame = decode(frame_json)?;
        self.0
            .open(fixed(routing)?, &frame)
            .map(|p| p.to_vec())
            .map_err(error)
    }
    pub fn transcript_hash(&self) -> Vec<u8> {
        self.0.transcript_hash.to_vec()
    }
}
#[wasm_bindgen]
pub fn verify_public_bundle(bundle_json: &str, pin: &[u8]) -> Result<(), JsValue> {
    decode::<SignedBundle>(bundle_json)?
        .verify(&fixed(pin)?)
        .map_err(error)
}
#[wasm_bindgen]
pub fn production_qualified() -> bool {
    crate::PRODUCTION_QUALIFIED
}

#[wasm_bindgen]
pub fn verify_peer_approval(
    approval_json: &str,
    host_json: &str,
    pin: &[u8],
    expected_json: &str,
    now: u64,
    current_epoch: u64,
) -> Result<(), JsValue> {
    let approval: crate::SignedPeerApproval = decode(approval_json)?;
    let host: SignedBundle = decode(host_json)?;
    let expected: crate::PeerApproval = decode(expected_json)?;
    approval
        .verify(&host, &fixed(pin)?, &expected, now, current_epoch)
        .map_err(error)
}
#[wasm_bindgen]
pub fn live_production_qualified() -> bool {
    crate::LIVE_PRODUCTION_QUALIFIED
}

#[wasm_bindgen]
pub fn pairing_fingerprint(
    account_id: &[u8],
    host_id: &[u8],
    device_id: &[u8],
    host_fingerprint: &[u8],
    device_fingerprint: &[u8],
    pairing_nonce: &[u8],
) -> Result<String, JsValue> {
    let digest = crate::pairing_fingerprint(
        &fixed(account_id)?,
        &fixed(host_id)?,
        &fixed(device_id)?,
        &fixed(host_fingerprint)?,
        &fixed(device_fingerprint)?,
        &fixed(pairing_nonce)?,
    );
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The approval must first be verified against trusted host and grant state.
#[wasm_bindgen]
pub fn validate_channel_approval(context_json: &str, approval_json: &str) -> Result<(), JsValue> {
    let context: ChannelContext = decode(context_json)?;
    let approval: crate::PeerApproval = decode(approval_json)?;
    context.validate_approval(&approval).map_err(error)
}
