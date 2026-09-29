use std::{
	borrow::Borrow,
	fmt::{self, Write},
};

use serde::{Deserialize, Serialize};

/// The `ResourceId` struct provides the unique identifier used to locate a stored resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(pub(crate) [u8; 16]);

impl From<&str> for ResourceId {
	fn from(value: &str) -> Self {
		let hash = md5::compute(value);
		Self(hash.0)
	}
}

impl ResourceId {
	/// Parses exactly 32 hexadecimal digits, accepting either letter case.
	pub fn from_uid_hex(value: &str) -> Option<Self> {
		let mut bytes = [0; 16];
		utils::hex::decode_into(value, &mut bytes)?;
		Some(Self(bytes))
	}

	pub fn to_hex(self) -> String {
		self.into()
	}
}

impl fmt::Display for ResourceId {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		utils::hex::encode_to(&self.0, formatter)
	}
}

impl From<ResourceId> for [u8; 16] {
	fn from(val: ResourceId) -> Self {
		val.0
	}
}

impl AsRef<[u8; 16]> for ResourceId {
	fn as_ref(&self) -> &[u8; 16] {
		&self.0
	}
}

impl Borrow<[u8; 16]> for &ResourceId {
	fn borrow(&self) -> &[u8; 16] {
		&self.0
	}
}

#[cfg(test)]
mod tests {
	use super::ResourceId;

	#[test]
	fn string_ids_use_stable_md5_bytes_and_hex_round_trip() {
		let id = ResourceId::from("hello");

		assert_eq!(id.to_hex(), "5d41402abc4b2a76b9719d911017c592");
		assert_eq!(ResourceId::from_uid_hex(&id.to_hex()), Some(id));
		assert_eq!(ResourceId::from_uid_hex("5D41402ABC4B2A76B9719D911017C592"), Some(id));
	}
}

impl From<ResourceId> for String {
	fn from(val: ResourceId) -> Self {
		let mut s = String::with_capacity(32);
		write!(s, "{val}").expect("Writing a resource ID to a String cannot fail");
		s
	}
}
