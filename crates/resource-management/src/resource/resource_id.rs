use std::{borrow::Borrow, fmt::{Debug, Write}};

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

impl Into<String> for ResourceId {
	fn into(self) -> String {
		let mut s = String::with_capacity(32);
		for byte in &self.0 {
			write!(s, "{:02x}", byte).unwrap();
		}
		s
	}
}