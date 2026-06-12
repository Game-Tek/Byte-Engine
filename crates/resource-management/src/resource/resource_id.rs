use std::{
	borrow::Borrow,
	fmt::{Debug, Write},
};

use serde::{Deserialize, Serialize};

/// A resource ID is a unique identifier for a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(pub(crate) [u8; 16]);

impl From<&str> for ResourceId {
	fn from(value: &str) -> Self {
		let hash = md5::compute(value);
		Self(hash.0)
	}
}

impl ResourceId {
	pub fn from_uid_hex(value: &str) -> Option<Self> {
		if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
			return None;
		}

		let mut bytes = [0; 16];
		for (index, byte) in bytes.iter_mut().enumerate() {
			let start = index * 2;
			*byte = u8::from_str_radix(&value[start..start + 2], 16).ok()?;
		}

		Some(Self(bytes))
	}

	pub fn to_hex(self) -> String {
		self.into()
	}
}

impl From<ResourceId> for [u8; 16] {
	fn from(val: ResourceId) -> Self {
		val.0
	}
}

impl AsRef<[u8; 16]> for ResourceId {
	fn as_ref(&self) -> &[u8; 16] {
		unsafe { std::mem::transmute(self) }
	}
}

impl Borrow<[u8; 16]> for &ResourceId {
	fn borrow(&self) -> &[u8; 16] {
		unsafe { std::mem::transmute(&self.0) }
	}
}

impl From<ResourceId> for String {
	fn from(val: ResourceId) -> Self {
		let mut s = String::with_capacity(32);
		for byte in &val.0 {
			write!(s, "{:02x}", byte).unwrap();
		}
		s
	}
}
