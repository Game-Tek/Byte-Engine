//! Encode bytes as hexadecimal text and decode it back, such as resource UIDs and shell-safe query cursors.

use std::fmt;

/// Writes `bytes` as lowercase hexadecimal digits, two per byte.
pub fn encode_to(bytes: &[u8], output: &mut impl fmt::Write) -> fmt::Result {
	const DIGITS: &[u8; 16] = b"0123456789abcdef";

	for byte in bytes {
		output.write_char(DIGITS[(byte >> 4) as usize] as char)?;
		output.write_char(DIGITS[(byte & 0x0f) as usize] as char)?;
	}

	Ok(())
}

/// Returns `bytes` as lowercase hexadecimal text.
pub fn encode(bytes: &[u8]) -> String {
	let mut output = String::with_capacity(bytes.len() * 2);
	encode_to(bytes, &mut output).expect("writing to a String cannot fail");
	output
}

/// Decodes hexadecimal text of either letter case into `output`, which must hold exactly half as many bytes.
///
/// Returns `None` when the lengths disagree or the text contains a non-hexadecimal character.
pub fn decode_into(value: &str, output: &mut [u8]) -> Option<()> {
	if value.len() != output.len().checked_mul(2)? {
		return None;
	}

	for (byte, digits) in output.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
		*byte = (decode_digit(digits[0])? << 4) | decode_digit(digits[1])?;
	}

	Some(())
}

/// Decodes hexadecimal text of either letter case into its bytes.
pub fn decode(value: &str) -> Option<Vec<u8>> {
	if !value.len().is_multiple_of(2) {
		return None;
	}

	let mut bytes = vec![0; value.len() / 2];
	decode_into(value, &mut bytes)?;
	Some(bytes)
}

/// Decodes one hexadecimal ASCII digit.
fn decode_digit(value: u8) -> Option<u8> {
	match value {
		b'0'..=b'9' => Some(value - b'0'),
		b'a'..=b'f' => Some(value - b'a' + 10),
		b'A'..=b'F' => Some(value - b'A' + 10),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::{decode, decode_into, encode};

	#[test]
	fn round_trips_all_byte_values_and_accepts_uppercase() {
		let bytes: Vec<u8> = (u8::MIN..=u8::MAX).collect();
		let encoded = encode(&bytes);

		assert_eq!(encoded.len(), bytes.len() * 2);
		assert_eq!(decode(&encoded), Some(bytes.clone()));
		assert_eq!(decode(&encoded.to_uppercase()), Some(bytes));
		assert_eq!(decode("0"), None);
		assert_eq!(decode("gg"), None);
	}

	#[test]
	fn fixed_size_decoding_rejects_the_wrong_length() {
		let mut output = [0; 2];

		assert_eq!(decode_into("abcd", &mut output), Some(()));
		assert_eq!(output, [0xab, 0xcd]);
		assert_eq!(decode_into("abc", &mut output), None);
		assert_eq!(decode_into("abcdef", &mut output), None);
	}
}
