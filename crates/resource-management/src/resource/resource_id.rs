use std::borrow::Borrow;

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

impl Into<[u8; 16]> for ResourceId {
	fn into(self) -> [u8; 16] {
		self.0
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

impl AsRef<str> for ResourceId {
	fn as_ref(&self) -> &str {
		unsafe { std::str::from_utf8_unchecked(&self.0) }
	}
}