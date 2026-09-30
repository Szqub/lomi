use crate::{Result, PROFILE};
use serde::{Deserialize, Serialize};
pub type Id = [u8; 16];
pub(crate) type Encoder = minicbor::Encoder<Vec<u8>>;
pub(crate) fn encoder() -> Encoder {
    minicbor::Encoder::new(Vec::new())
}
pub(crate) fn bytes(e: &mut Encoder, b: &[u8]) {
    e.bytes(b).unwrap();
}
pub(crate) fn number(e: &mut Encoder, n: u64) {
    e.u64(n).unwrap();
}
pub(crate) fn domain(e: &mut Encoder, n: u64, name: &str) {
    e.array(n).unwrap();
    e.str(PROFILE).unwrap();
    e.str(name).unwrap();
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Host,
    Device,
}
impl Role {
    pub(crate) fn code(self) -> u64 {
        match self {
            Self::Host => 1,
            Self::Device => 2,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelContext {
    pub version: u32,
    pub account_id: Id,
    pub host_id: Id,
    pub device_id: Id,
    pub initiator_role: Role,
    pub responder_role: Role,
}
impl ChannelContext {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.initiator_role != Role::Device
            || self.responder_role != Role::Host
        {
            return Err("unsupported channel context".into());
        }
        Ok(())
    }
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut e = encoder();
        domain(&mut e, 8, "channel-context");
        number(&mut e, u64::from(self.version));
        bytes(&mut e, &self.account_id);
        bytes(&mut e, &self.host_id);
        bytes(&mut e, &self.device_id);
        number(&mut e, self.initiator_role.code());
        number(&mut e, self.responder_role.code());
        Ok(e.into_writer())
    }
}
