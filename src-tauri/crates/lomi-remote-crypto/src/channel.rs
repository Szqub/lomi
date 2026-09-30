use crate::{wire, ChannelContext, Identity, Result, Role, SignedBundle, MAX_PLAINTEXT};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;
const NOISE: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
const MAX_HANDSHAKE: usize = 512;
const MAX_FRAMES: u64 = 1_000_000;
const HEADER: usize = 24;
pub struct Handshake {
    state: Option<snow::HandshakeState>,
    peer_static: [u8; 32],
}
pub struct Channel {
    state: Option<snow::TransportState>,
    send_seq: u64,
    receive_seq: u64,
    pub transcript_hash: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportFrame {
    pub sequence: u64,
    pub routing: [u8; 16],
    pub ciphertext: Vec<u8>,
}
impl Handshake {
    pub fn new(
        initiator: bool,
        identity: &Identity,
        context: &ChannelContext,
        peer: &SignedBundle,
        pinned_peer: &[u8; 32],
    ) -> Result<Self> {
        context.validate()?;
        peer.verify(pinned_peer)?;
        let local = &identity.signed.bundle;
        let local_role = if initiator { Role::Device } else { Role::Host };
        let peer_role = if initiator { Role::Host } else { Role::Device };
        let subject = |role| match role {
            Role::Host => context.host_id,
            Role::Device => context.device_id,
        };
        if local.account_id != context.account_id
            || peer.bundle.account_id != context.account_id
            || local.role != local_role
            || peer.bundle.role != peer_role
            || local.subject_id != subject(local_role)
            || peer.bundle.subject_id != subject(peer_role)
        {
            return Err("channel identity/context mismatch".into());
        }
        let (device, host) = if initiator {
            (&identity.signed, peer)
        } else {
            (peer, &identity.signed)
        };
        let mut e = wire::encoder();
        wire::domain(&mut e, 5, "noise-prologue");
        wire::bytes(&mut e, &context.canonical()?);
        wire::bytes(&mut e, &device.bundle.fingerprint()?);
        wire::bytes(&mut e, &host.bundle.fingerprint()?);
        let prologue = e.into_writer();
        let builder = snow::Builder::new(NOISE.parse().map_err(|_| "Noise profile unavailable")?)
            .local_private_key(identity.channel_secret.as_ref())
            .map_err(|_| "Noise key rejected")?
            .prologue(&prologue)
            .map_err(|_| "Noise prologue rejected")?;
        let state = if initiator {
            builder.build_initiator()
        } else {
            builder.build_responder()
        }
        .map_err(|_| "Noise construction failed")?;
        Ok(Self {
            state: Some(state),
            peer_static: peer.bundle.channel_key,
        })
    }
    pub fn write(&mut self) -> Result<Vec<u8>> {
        let mut state = self.state.take().ok_or("handshake closed")?;
        let mut output = vec![0u8; MAX_HANDSHAKE];
        let count = state
            .write_message(&[], &mut output)
            .map_err(|_| "handshake write rejected")?;
        output.truncate(count);
        self.state = Some(state);
        Ok(output)
    }
    pub fn read(&mut self, message: &[u8]) -> Result<()> {
        let mut state = self.state.take().ok_or("handshake closed")?;
        if message.len() > MAX_HANDSHAKE {
            return Err("handshake frame limit".into());
        }
        let mut output = Zeroizing::new(vec![0u8; MAX_HANDSHAKE]);
        let count = state
            .read_message(message, &mut output)
            .map_err(|_| "handshake authentication failed")?;
        if count != 0 {
            return Err("unexpected handshake application payload".into());
        }
        if state
            .get_remote_static()
            .is_some_and(|key| key != self.peer_static)
        {
            return Err("Noise static pin mismatch".into());
        }
        self.state = Some(state);
        Ok(())
    }
    pub fn finish(mut self) -> Result<Channel> {
        let state = self.state.take().ok_or("handshake closed")?;
        if !state.is_handshake_finished()
            || state.get_remote_static() != Some(self.peer_static.as_slice())
        {
            return Err("handshake incomplete or static pin mismatch".into());
        }
        let transcript_hash = state
            .get_handshake_hash()
            .try_into()
            .map_err(|_| "unexpected transcript hash size")?;
        let transport = state
            .into_transport_mode()
            .map_err(|_| "transport unavailable")?;
        Ok(Channel {
            state: Some(transport),
            send_seq: 0,
            receive_seq: 0,
            transcript_hash,
        })
    }
}
impl Channel {
    /// Mutable ownership serializes Snow's send nonce. Channel is deliberately not Clone.
    pub fn seal(&mut self, routing: [u8; 16], plaintext: &[u8]) -> Result<TransportFrame> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err("plaintext limit".into());
        }
        let mut state = self.state.take().ok_or("channel closed")?;
        if self.send_seq >= MAX_FRAMES {
            return Err("channel lifetime limit; new handshake required".into());
        }
        let mut input = Zeroizing::new(Vec::with_capacity(HEADER + plaintext.len()));
        input.extend_from_slice(&self.send_seq.to_be_bytes());
        input.extend_from_slice(&routing);
        input.extend_from_slice(plaintext);
        let mut ciphertext = vec![0; input.len() + 16];
        let count = state
            .write_message(&input, &mut ciphertext)
            .map_err(|_| "channel encryption failed")?;
        ciphertext.truncate(count);
        let frame = TransportFrame {
            sequence: self.send_seq,
            routing,
            ciphertext,
        };
        self.send_seq += 1;
        self.state = Some(state);
        Ok(frame)
    }
    /// Any authentication/order/context failure closes the receive channel.
    pub fn open(
        &mut self,
        expected_routing: [u8; 16],
        frame: &TransportFrame,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let mut state = self.state.take().ok_or("channel closed")?;
        if frame.ciphertext.len() < HEADER + 16
            || frame.ciphertext.len() > MAX_PLAINTEXT + HEADER + 16
            || frame.sequence != self.receive_seq
            || frame.routing != expected_routing
            || self.receive_seq >= MAX_FRAMES
        {
            return Err("transport limit, order, or routing mismatch".into());
        }
        let mut plaintext = Zeroizing::new(vec![0u8; frame.ciphertext.len()]);
        let count = state
            .read_message(&frame.ciphertext, &mut plaintext)
            .map_err(|_| "transport authentication failed")?;
        if count < HEADER
            || plaintext[..8] != frame.sequence.to_be_bytes()
            || plaintext[8..HEADER] != expected_routing
        {
            return Err("encrypted transport context mismatch".into());
        }
        plaintext.drain(..HEADER);
        plaintext.truncate(count - HEADER);
        self.receive_seq += 1;
        self.state = Some(state);
        Ok(plaintext)
    }
}
