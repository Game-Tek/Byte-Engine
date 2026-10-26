//! Human-readable storage amounts for log and error messages.

use std::fmt;

/// Binary units from largest to smallest, each paired with its size in bytes.
const UNITS: [(&str, u64); 6] = [
	("EiB", 1 << 60),
	("PiB", 1 << 50),
	("TiB", 1 << 40),
	("GiB", 1 << 30),
	("MiB", 1 << 20),
	("KiB", 1 << 10),
];

/// The `ByteSize` struct lets a log or error message show a storage amount, such as a capacity, budget, or memory use,
/// in the largest binary unit that keeps it at one or more, so `67108864` reads as `64 MiB`.
///
/// Wrap the byte count where the message is formatted, as in `format!("{}", ByteSize::from(capacity))`. It writes
/// straight into the formatter and allocates nothing. Exact sizes a reader compares byte for byte, such as offsets
/// or layout mismatches, stay plain integers.
#[derive(Clone, Copy, Debug)]
pub struct ByteSize(pub u64);

impl From<usize> for ByteSize {
	fn from(bytes: usize) -> Self {
		Self(bytes as u64)
	}
}

impl fmt::Display for ByteSize {
	/// Writes the amount with one rounded decimal, dropping a trailing `.0`.
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		// Whole bytes are exact, so they never round up into the next unit.
		if self.0 < 1 << 10 {
			return write!(formatter, "{} B", self.0);
		}
		let bytes = u128::from(self.0);
		// Choose the unit after rounding, so 1,048,575 bytes reads as `1 MiB` instead of `1024 KiB`.
		let (unit, tenths) = UNITS
			.iter()
			.map(|&(unit, size)| {
				let size = u128::from(size);
				(unit, (bytes * 10 + size / 2) / size)
			})
			.find(|&(_, tenths)| tenths >= 10)
			.expect("Amounts of 1 KiB or more always reach one KiB");
		if tenths % 10 == 0 {
			write!(formatter, "{} {unit}", tenths / 10)
		} else {
			write!(formatter, "{}.{} {unit}", tenths / 10, tenths % 10)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::ByteSize;

	#[test]
	fn amounts_take_the_largest_unit_that_keeps_them_at_one_or_more() {
		assert_eq!(ByteSize(0).to_string(), "0 B");
		assert_eq!(ByteSize(1023).to_string(), "1023 B");
		assert_eq!(ByteSize(1024).to_string(), "1 KiB");
		assert_eq!(ByteSize(64 * 1024).to_string(), "64 KiB");
		assert_eq!(ByteSize(64 << 20).to_string(), "64 MiB");
		assert_eq!(ByteSize(3 << 30).to_string(), "3 GiB");
		assert_eq!(ByteSize(5 << 40).to_string(), "5 TiB");
		assert_eq!(ByteSize(u64::MAX).to_string(), "16 EiB");
	}

	#[test]
	fn fractional_amounts_show_one_rounded_decimal() {
		assert_eq!(ByteSize(1536).to_string(), "1.5 KiB");
		assert_eq!(ByteSize(70_000).to_string(), "68.4 KiB");
		assert_eq!(ByteSize((1 << 20) - 1).to_string(), "1 MiB");
		assert_eq!(ByteSize((1 << 20) + (1 << 19)).to_string(), "1.5 MiB");
	}
}
