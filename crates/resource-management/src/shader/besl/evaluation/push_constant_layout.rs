//! Checks that a push-constant block has the same byte layout on every backend.

use std::{collections::HashSet, num::NonZeroUsize, ops::Range};

use super::reflection::{StorageLayoutTarget, checked_align_up, primitive_storage_layout, reflected_storage_type_layout};

const PUSH_CONSTANT_DOCUMENTATION: &str = "https://byte-engine.0x44491229.dev/docs/reference/besl/interface#push-constants";

/// The size of one HLSL constant-buffer register.
const REGISTER: usize = 16;

/// Rejects a push-constant block whose members would sit at different byte offsets, or take different sizes, on some
/// backend.
///
/// The CPU writes a push-constant block with scalar layout. GLSL declares the block `scalar`, so Vulkan reads it as
/// written, and Metal packs vector members, so it does too. HLSL reads push constants through a constant buffer, which
/// counts in 16-byte registers: a value can't cross a register boundary, and arrays start on a new register. The check
/// runs on every platform, so a block that only one backend would misread fails everywhere.
/// [`super::ProgramEvaluation`] calls it while it reflects a program.
pub(super) fn validate_push_constant_layout(members: &[besl::NodeReference]) -> Result<(), String> {
	// Every member before the current one matched on all backends, so they agree on where the next one may start.
	let mut offset = 0;
	for member in members {
		let member = member.borrow();
		let besl::Nodes::Member { name, r#type, count } = member.node() else {
			return Err(
				"Unsupported push-constant member. The most likely cause is that a push-constant block contains a node other than a named member."
					.to_string(),
			);
		};
		let count = count.map(NonZeroUsize::get);
		let cpu = storage_range(r#type, count, offset, StorageLayoutTarget::GlslScalar)?;
		let metal = storage_range(r#type, count, offset, StorageLayoutTarget::Msl)?;
		let dx12 = constant_buffer_range(r#type, count, offset, name)?;
		if cpu.start != metal.start || cpu.start != dx12.start {
			return Err(format!(
				"Push-constant member `{name}` starts at byte {} on the CPU and Vulkan, {} on Metal, and {} on DX12. The most likely cause is that a value crosses a 16-byte boundary, or that an array doesn't start on one; reorder the members or add explicit padding. See {PUSH_CONSTANT_DOCUMENTATION}.",
				cpu.start, metal.start, dx12.start
			));
		}
		if cpu.len() != metal.len() || cpu.len() != dx12.len() {
			return Err(format!(
				"Push-constant member `{name}` takes {} bytes on the CPU and Vulkan, {} on Metal, and {} on DX12. The most likely cause is an array of elements smaller than 16 bytes, or a type whose size differs between backends, such as `bool` or `u8`; use `vec4` elements or `u32` instead. See {PUSH_CONSTANT_DOCUMENTATION}.",
				cpu.len(),
				metal.len(),
				dx12.len()
			));
		}
		offset = cpu.end;
	}
	Ok(())
}

/// Places a member at or after `offset` with the rules a backend uses for buffer records.
fn storage_range(
	r#type: &besl::NodeReference,
	count: Option<usize>,
	offset: usize,
	target: StorageLayoutTarget,
) -> Result<Range<usize>, String> {
	let layout = reflected_storage_type_layout(r#type, target, &mut HashSet::new())?;
	let start = checked_align_up(offset, layout.alignment)?;
	let size = checked_align_up(layout.size, layout.alignment)?.saturating_mul(count.unwrap_or(1));
	Ok(start..start.saturating_add(size))
}

/// Places a member at or after `offset` with HLSL constant-buffer packing.
fn constant_buffer_range(
	r#type: &besl::NodeReference,
	count: Option<usize>,
	offset: usize,
	name: &str,
) -> Result<Range<usize>, String> {
	let r#type = r#type.borrow();
	let type_name = r#type.get_name().unwrap_or("unknown");
	// Structs and matrices other than `mat4f` leave register tails empty, so their constant-buffer layout can't match
	// the scalar one.
	let layout = primitive_storage_layout(type_name, StorageLayoutTarget::Hlsl)
		.filter(|_| !type_name.starts_with("mat") || type_name == "mat4f")
		.ok_or_else(|| {
			format!(
				"Push-constant member `{name}` has type `{type_name}`, which DX12 can't read with the scalar layout. The most likely cause is a struct, a resource handle, or a matrix other than `mat4f` in a push constant; use scalars, vectors, `mat4f`, or arrays of them, or move the value to a buffer binding. See {PUSH_CONSTANT_DOCUMENTATION}."
			)
		})?;
	let (start, size) = match count {
		// Each array element starts on a new register; the last one may share its register with the next value.
		Some(count) => (
			checked_align_up(offset, REGISTER)?,
			checked_align_up(layout.size, REGISTER)?
				.saturating_mul(count - 1)
				.saturating_add(layout.size),
		),
		// A value keeps its component alignment but can't cross a register boundary, so a `mat4f` always starts on one.
		None => {
			let start = checked_align_up(offset, layout.alignment)?;
			let crosses = start / REGISTER != (start + layout.size - 1) / REGISTER;
			(if crosses { checked_align_up(start, REGISTER)? } else { start }, layout.size)
		}
	};
	Ok(start..start.saturating_add(size))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Builds a push-constant member list from `(name, type, array count)` triples over the built-in type registry.
	fn members(fields: &[(&str, &str, Option<usize>)]) -> Vec<besl::NodeReference> {
		let root = besl::Node::root();
		fields
			.iter()
			.map(|(name, type_name, count)| {
				let r#type = root.get_child(type_name).expect("Expected a built-in type");
				match count {
					Some(count) => besl::Node::array(name, r#type, *count),
					None => besl::Node::member(name, r#type).into(),
				}
			})
			.collect()
	}

	#[test]
	fn blocks_every_backend_reads_alike_are_accepted() {
		for block in [
			// The particle draw block: a vec3f after a mat4f, then a scalar in the same register.
			vec![
				("view_projection", "mat4f", None),
				("camera_position", "vec3f", None),
				("exposure", "f32", None),
			],
			// The UI blocks: vectors on their natural offsets, and an array of 16-byte elements.
			vec![("viewport", "vec2f", None), ("first", "u32", None)],
			vec![
				("origin", "vec2u", None),
				("extent", "vec2u", None),
				("weights", "vec4f", Some(2)),
			],
			// Vectors at 4-byte offsets that stay inside one register.
			vec![("scale", "f32", None), ("offset", "vec2f", None), ("tint", "vec2f16", None)],
		] {
			assert_eq!(validate_push_constant_layout(&members(&block)), Ok(()), "{block:?}");
		}
	}

	#[test]
	fn blocks_some_backend_reads_differently_are_rejected() {
		for (block, expected) in [
			(
				vec![("scale", "f32", None), ("bias", "f32", None), ("offset", "vec3f", None)],
				"`offset` starts at byte 8 on the CPU and Vulkan, 8 on Metal, and 16 on DX12",
			),
			(
				vec![("index", "u32", None), ("transform", "mat4f", None)],
				"`transform` starts at byte 4 on the CPU and Vulkan, 16 on Metal, and 16 on DX12",
			),
			(
				vec![("weights", "f32", Some(4)), ("count", "u32", None)],
				"`weights` takes 16 bytes on the CPU and Vulkan, 16 on Metal, and 52 on DX12",
			),
			(
				vec![("flag", "bool", None)],
				"`flag` takes 4 bytes on the CPU and Vulkan, 1 on Metal, and 4 on DX12",
			),
			(
				vec![("flag", "u8", None)],
				"`flag` takes 1 bytes on the CPU and Vulkan, 1 on Metal, and 4 on DX12",
			),
			(
				vec![("affine", "mat4x3f", None)],
				"`affine` has type `mat4x3f`, which DX12 can't read",
			),
		] {
			let error = validate_push_constant_layout(&members(&block)).expect_err("Expected the block to be rejected");

			assert!(error.contains(expected), "{block:?}: {error}");
			assert!(error.contains(PUSH_CONSTANT_DOCUMENTATION), "{error}");
		}
	}
}
