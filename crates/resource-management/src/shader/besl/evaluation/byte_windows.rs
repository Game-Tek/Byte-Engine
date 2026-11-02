//! Plans the 2-byte windows that hold `u8` members on DX12.
//!
//! HLSL has no 1-byte member type, so DX12 stores each `u8` member of a struct, buffer block, or push-constant block
//! in the `uint16_t` window that covers its scalar offset, and reads and writes its byte with shifts. The windows sit at
//! even offsets, so every backend agrees on the bytes as long as a window's other byte is another `u8` member or
//! padding. [`validate_byte_windows`] checks that, on every platform, for every layout reflection reports.

use std::{collections::HashSet, ops::Range};

use super::reflection::{StorageLayoutTarget, checked_align_up, primitive_storage_layout, reflected_storage_type_layout};

const STORED_RECORDS_DOCS_PATH: &str = "reference/besl/interface#stored-records";

/// The `ByteWindow` struct is one `uint16_t` that DX12 declares in place of the `u8` members it holds.
pub(crate) struct ByteWindow {
	/// The window's scalar byte offset, which is always even.
	pub(crate) offset: usize,
	/// The `u8` members the window holds, in member order, each with the bit shift of its byte in the window.
	pub(crate) bytes: Vec<(besl::NodeReference, u32)>,
}

/// Places every single `u8` member of `members` in the window that covers its scalar offset, in member order.
///
/// Returns an error when a member has no storage layout, such as a resource handle in a struct that only locals use.
pub(crate) fn byte_windows(members: &[besl::NodeReference]) -> Result<Vec<ByteWindow>, String> {
	let (ranges, _) = scalar_member_ranges(members)?;
	let mut windows: Vec<ByteWindow> = Vec::new();
	for (member, range) in members.iter().zip(ranges) {
		if !is_single_u8(member) {
			continue;
		}
		let offset = range.start & !1;
		let shift = if range.start % 2 == 0 { 0 } else { 8 };
		// Offsets grow in member order, so a window's bytes are always next to each other in the list.
		match windows.last_mut() {
			Some(window) if window.offset == offset => window.bytes.push((member.clone(), shift)),
			_ => windows.push(ByteWindow {
				offset,
				bytes: vec![(member.clone(), shift)],
			}),
		}
	}
	Ok(windows)
}

/// Rejects a member list, or a struct it nests, whose `u8` windows DX12 can't store at their scalar offsets: a window
/// whose other byte belongs to a member that isn't a `u8`, or lies past the end of the list's storage.
pub(super) fn validate_byte_windows(members: &[besl::NodeReference]) -> Result<(), String> {
	validate_members(members, &mut HashSet::new())
}

fn validate_members(members: &[besl::NodeReference], validated: &mut HashSet<besl::NodeReference>) -> Result<(), String> {
	// Nested structs are emitted as their own HLSL structs, so their windows must hold up on their own too.
	for member in members {
		let r#type = match member.borrow().node() {
			besl::Nodes::Member { r#type, .. } => r#type.clone(),
			_ => continue,
		};
		let fields = match r#type.borrow().node() {
			besl::Nodes::Struct { name, fields, .. }
				if !fields.is_empty() && primitive_storage_layout(name, StorageLayoutTarget::GlslScalar).is_none() =>
			{
				fields.clone()
			}
			_ => continue,
		};
		if validated.insert(r#type) {
			validate_members(&fields, validated)?;
		}
	}

	let (ranges, size) = scalar_member_ranges(members)?;
	for window in byte_windows(members)? {
		for byte in window.offset..window.offset + 2 {
			// A byte the window's own `u8` members hold is fine; any other byte must be padding inside the layout.
			if window
				.bytes
				.iter()
				.any(|(_, shift)| window.offset + *shift as usize / 8 == byte)
			{
				continue;
			}
			let (member, _) = &window.bytes[0];
			let name = member.borrow().get_name().unwrap_or_default().to_string();
			if byte >= size {
				return Err(format!(
					"`u8` member `{name}` ends a layout of {size} bytes. DX12 stores `u8` members in 2-byte windows, so the layout would take {} bytes there. Add a `u8` member after it, or a wider member, so the layout's size is even. See {}.",
					size + 1,
					crate::online_docs_url(STORED_RECORDS_DOCS_PATH)
				));
			}
			if let Some(index) = ranges.iter().position(|range| range.contains(&byte)) {
				let neighbor = members[index].borrow().get_name().unwrap_or_default().to_string();
				return Err(format!(
					"`u8` member `{name}` shares 2 bytes with member `{neighbor}`. DX12 stores `u8` members in 2-byte windows, which can only hold other single `u8` members. Move `{name}` next to another `u8` member, or put a member with 2-byte alignment between them. See {}.",
					crate::online_docs_url(STORED_RECORDS_DOCS_PATH)
				));
			}
		}
	}
	Ok(())
}

/// Places `members` with the scalar layout the CPU writes: the byte range of each member, and the size of the whole
/// list rounded up to its alignment.
fn scalar_member_ranges(members: &[besl::NodeReference]) -> Result<(Vec<Range<usize>>, usize), String> {
	let mut ranges = Vec::with_capacity(members.len());
	let mut offset = 0usize;
	let mut alignment = 1usize;
	for member in members {
		let member = member.borrow();
		let besl::Nodes::Member { r#type, count, .. } = member.node() else {
			return Err(
				"Unsupported storage member. The most likely cause is that a layout contains a node other than a named member."
					.to_string(),
			);
		};
		let layout = reflected_storage_type_layout(r#type, StorageLayoutTarget::GlslScalar, &mut HashSet::new())?;
		let stride = checked_align_up(layout.size, layout.alignment)?;
		let start = checked_align_up(offset, layout.alignment)?;
		let end = stride
			.checked_mul(count.map_or(1, std::num::NonZeroUsize::get))
			.and_then(|size| start.checked_add(size))
			.ok_or_else(|| {
				"Storage layout overflows. The most likely cause is an excessively large fixed array.".to_string()
			})?;
		ranges.push(start..end);
		offset = end;
		alignment = alignment.max(layout.alignment);
	}
	Ok((ranges, checked_align_up(offset, alignment)?))
}

/// Whether `member` is one `u8`, the only member DX12 stores in a window.
fn is_single_u8(member: &besl::NodeReference) -> bool {
	matches!(member.borrow().node(), besl::Nodes::Member { r#type, count: None, .. } if r#type.borrow().get_name() == Some("u8"))
}
