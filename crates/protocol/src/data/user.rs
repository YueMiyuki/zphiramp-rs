use bytes::{Buf, BufMut};

use crate::error::ProtocolError;
use crate::wire::{Decode, Encode, decode_string_default, encode_string, get_bool, get_i32_le};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserProfile {
    pub user_id: i32,

    pub user_name: String,
}

impl UserProfile {
    pub fn new(user_id: i32, user_name: impl Into<String>) -> Self {
        Self {
            user_id,
            user_name: user_name.into(),
        }
    }
}

impl Encode for UserProfile {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_i32_le(self.user_id);
        encode_string(buf, &self.user_name);
    }
}

impl Decode for UserProfile {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            user_id: get_i32_le(buf)?,
            user_name: decode_string_default(buf)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FullUserProfile {
    pub user_profile: UserProfile,

    pub monitor: bool,
}

impl FullUserProfile {
    pub const MIN_WIRE_SIZE: usize = 6;

    pub fn new(user_id: i32, user_name: impl Into<String>, monitor: bool) -> Self {
        Self {
            user_profile: UserProfile::new(user_id, user_name),
            monitor,
        }
    }

    pub fn user_id(&self) -> i32 {
        self.user_profile.user_id
    }

    pub fn user_name(&self) -> &str {
        &self.user_profile.user_name
    }

    pub fn from_lists(users: &[UserProfile], monitors: &[UserProfile]) -> Vec<Self> {
        let mut out = Vec::with_capacity(users.len() + monitors.len());
        out.extend(users.iter().map(|u| Self {
            user_profile: u.clone(),
            monitor: false,
        }));
        out.extend(monitors.iter().map(|u| Self {
            user_profile: u.clone(),
            monitor: true,
        }));
        out
    }
}

impl Encode for FullUserProfile {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        self.user_profile.encode(buf);
        buf.put_u8(u8::from(self.monitor));
    }
}

impl Decode for FullUserProfile {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            user_profile: UserProfile::decode(buf)?,
            monitor: get_bool(buf)?,
        })
    }
}
