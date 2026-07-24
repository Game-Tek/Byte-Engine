use std::{
	borrow::Borrow,
	fmt::{Debug, Write},
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
	use std::borrow::Borrow;

	use super::ResourceId;

	#[test]
	fn string_ids_use_stable_md5_bytes_and_hex_round_trip() {
		let id = ResourceId::from("hello");
		assert_eq!(id.to_hex(), "5d41402abc4b2a76b9719d911017c592");
		assert_eq!(ResourceId::from_uid_hex(&id.to_hex()), Some(id));
		assert_eq!(ResourceId::from_uid_hex("5D41402ABC4B2A76B9719D911017C592"), Some(id));
	}

	#[test]
	fn hex_parser_rejects_wrong_length_and_non_hex_input() {
		assert_eq!(ResourceId::from_uid_hex("abc"), None);
		assert_eq!(ResourceId::from_uid_hex("zz41402abc4b2a76b9719d911017c592"), None);
	}

	#[test]
	fn byte_views_reference_the_exact_identifier_storage() {
		let id = ResourceId::from_uid_hex("00112233445566778899aabbccddeeff").unwrap();
		let expected = [
			0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
		];

		assert_eq!(*id.as_ref(), expected);
		assert_eq!(<[u8; 16]>::from(id), expected);
		assert_eq!(<&ResourceId as Borrow<[u8; 16]>>::borrow(&&id), &expected);
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
