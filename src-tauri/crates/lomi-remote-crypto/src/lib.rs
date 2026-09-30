//! Unqualified S01 prototype. Never enables production remote access.
//! Snow's internal secret copies are not proven to zeroize on drop.
#[cfg(feature = "browser")]
mod browser;
mod channel;
mod identity;
mod mailbox;
mod wire;
pub use channel::{Channel, Handshake, TransportFrame};
pub use identity::{Identity, PublicBundle, SignedBundle};
pub use mailbox::{Envelope, EnvelopeMetadata, MailboxPolicy};
pub use wire::{ChannelContext, Role};
pub type Result<T> = std::result::Result<T, String>;
pub const PROFILE: &str = "lomi-s01-unqualified-v1";
pub const MAX_PLAINTEXT: usize = 16 * 1024;
pub const MAX_MAILBOX_PAYLOAD: usize = 8 * 1024;
pub const MAX_JSON: usize = 128 * 1024;
pub const PRODUCTION_QUALIFIED: bool = false;
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom04::fill(&mut bytes).map_err(|_| "entropy unavailable")?;
    Ok(bytes)
}
