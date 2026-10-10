//! Reads the top-level usages out of a raw HID report descriptor.
//!
//! Linux exposes each device's report descriptor in sysfs but not its usages, so the Linux scan parses them here
//! before it reports a device.

use super::Usage;

/// Returns the usage of every top-level collection in `descriptor`, in order.
///
/// Malformed or truncated items end the iteration instead of failing.
pub(super) fn top_level_usages(descriptor: &[u8]) -> TopLevelUsages<'_> {
	TopLevelUsages {
		descriptor,
		usage_page: 0,
		usage: None,
		depth: 0,
	}
}

/// The `TopLevelUsages` struct walks a report descriptor without allocating; build it with [`top_level_usages`].
pub(super) struct TopLevelUsages<'a> {
	descriptor: &'a [u8],
	/// The current Usage Page global item.
	usage_page: u16,
	/// The first Usage local item since the last main item, with the page an extended usage carries.
	usage: Option<(Option<u16>, u16)>,
	/// How many collections are open.
	depth: u32,
}

impl Iterator for TopLevelUsages<'_> {
	type Item = Usage;

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			let (&prefix, rest) = self.descriptor.split_first()?;

			// A long item carries its data size in the next byte and has no meaning for usages.
			if prefix == 0xFE {
				let size = *rest.first()? as usize;
				self.descriptor = rest.get(2 + size..)?;
				continue;
			}

			let size = match prefix & 0x03 {
				3 => 4,
				size => size as usize,
			};
			let data = rest.get(..size)?;
			self.descriptor = &rest[size..];
			let value = data.iter().rev().fold(0u32, |value, &byte| (value << 8) | byte as u32);

			match prefix & 0xFC {
				// Usage Page (global).
				0x04 => self.usage_page = value as u16,
				// Usage (local). Only the first usage before a collection names it; a four-byte usage carries its page.
				0x08 => {
					if self.usage.is_none() {
						let page = (size == 4).then_some((value >> 16) as u16);
						self.usage = Some((page, value as u16));
					}
				}
				// Collection (main).
				0xA0 => {
					let usage = self.usage.take();
					self.depth += 1;
					if self.depth == 1
						&& let Some((page, usage)) = usage
					{
						return Some(Usage {
							page: page.unwrap_or(self.usage_page),
							usage,
						});
					}
				}
				// End Collection (main).
				0xC0 => {
					self.depth = self.depth.saturating_sub(1);
					self.usage = None;
				}
				// Input, Output, and Feature main items also end the local item set.
				0x80 | 0x90 | 0xB0 => self.usage = None,
				_ => {}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const fn usage(page: u16, usage: u16) -> Usage {
		Usage { page, usage }
	}

	#[test]
	fn reports_each_top_level_collection() {
		// Usage Page (Generic Desktop), Usage (Game Pad), Collection (Application),
		//   Usage (Pointer), Collection (Physical), End Collection,
		// End Collection,
		// Usage Page (Vendor 0xFF00), Usage (0x01), Collection (Application), End Collection
		let descriptor = [
			0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0x09, 0x01, 0xA1, 0x00, 0xC0, 0xC0, 0x06, 0x00, 0xFF, 0x09, 0x01, 0xA1, 0x01,
			0xC0,
		];

		let usages: Vec<_> = top_level_usages(&descriptor).collect();

		assert_eq!(usages, [usage(0x01, 0x05), usage(0xFF00, 0x01)]);
	}

	#[test]
	fn extended_usage_carries_its_page() {
		// Usage (Generic Desktop: Joystick) as a four-byte usage, Collection (Application), End Collection
		let descriptor = [0x0B, 0x04, 0x00, 0x01, 0x00, 0xA1, 0x01, 0xC0];

		let usages: Vec<_> = top_level_usages(&descriptor).collect();

		assert_eq!(usages, [usage(0x01, 0x04)]);
	}

	#[test]
	fn truncated_descriptor_stops_early() {
		// Usage Page (Generic Desktop), Usage (Game Pad), Collection (Application), then a cut-off Usage Page.
		let descriptor = [0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0x06, 0x00];

		let usages: Vec<_> = top_level_usages(&descriptor).collect();

		assert_eq!(usages, [usage(0x01, 0x05)]);
	}
}
