use std::{cell::RefCell, collections::HashSet};

/// The `BindingUsage` struct provides reflection metadata for one binding used by a BESL program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingUsage {
	pub name: String,
	pub kind: BindingKind,
	pub count: u32,
	pub slot: u32,
	pub buffer_stride: Option<u32>,
	pub read: bool,
	pub write: bool,
}

/// The `BindingKind` enum identifies the descriptor category declared by a BESL binding.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum BindingKind {
	/// A structured storage buffer. Read-only access does not change the descriptor category.
	StorageBuffer,
	CombinedImageSampler {
		view: TextureView,
	},
	StorageImage,
}

/// The `TextureView` enum identifies the texture shape required by a BESL sampled-image binding.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum TextureView {
	Texture2D,
	Texture2DArray,
	TextureCube,
	TextureCubeArray,
	Texture3D,
}

/// The `BindingCollectionState` struct keeps reflection traversal aligned with graph identity deduplication.
struct BindingCollectionState {
	visited: utils::hash::HashSet<besl::NodeReference>,
	error: Option<String>,
}

/// The `StorageLayoutTarget` enum identifies the storage rules used by the active shader backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StorageLayoutTarget {
	Hlsl,
	Msl,
	GlslScalar,
}

impl StorageLayoutTarget {
	/// Selects the layout model that matches the backend compiled for this target.
	pub(super) const fn current() -> Self {
		if cfg!(target_vendor = "apple") {
			Self::Msl
		} else if cfg!(target_os = "windows") {
			Self::Hlsl
		} else {
			Self::GlslScalar
		}
	}
}

/// The `StorageLayout` struct records the byte size and alignment of one emitted shader type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StorageLayout {
	pub(super) size: usize,
	pub(super) alignment: usize,
}

/// Reflects the byte stride used when one storage-buffer element is addressed.
fn reflected_storage_buffer_stride(members: &[besl::NodeReference]) -> Result<u32, String> {
	reflected_storage_buffer_stride_for_target(members, StorageLayoutTarget::current())
}

/// Reflects the stride of one element in an array buffer.
fn reflected_array_buffer_stride(element: &besl::NodeReference) -> Result<u32, String> {
	reflected_array_buffer_stride_for_target(element, StorageLayoutTarget::current())
}

/// Reflects an array-buffer element, fixed-size or runtime-length, using the selected backend's emitted layout.
pub(super) fn reflected_array_buffer_stride_for_target(
	element: &besl::NodeReference,
	target: StorageLayoutTarget,
) -> Result<u32, String> {
	// User structs are emitted as packed element structs. Built-in records such as `vec3f` and `mat4x3f` have fields
	// too, but are emitted as packed scalar or vector types.
	let user_struct_fields = {
		let element = element.borrow();
		match element.node() {
			besl::Nodes::Struct { name, fields, .. }
				if !fields.is_empty() && primitive_storage_layout(name, target).is_none() =>
			{
				Some(fields.clone())
			}
			_ => None,
		}
	};
	let mut visiting = HashSet::new();
	let layout = if let Some(fields) = user_struct_fields {
		reflected_storage_members_layout(&fields, target, &mut visiting)?
	} else if target == StorageLayoutTarget::Hlsl
		&& element
			.borrow()
			.get_name()
			.and_then(crate::shader::besl::backends::hlsl::hlsl_narrow_element)
			.is_some()
	{
		// DX12 exposes narrow arrays as atomically addressable 32-bit words,
		// even though native u16 values remain two bytes in every other HLSL layout.
		StorageLayout { size: 4, alignment: 4 }
	} else {
		reflected_storage_type_layout(element, target, &mut visiting)?
	};
	let size = checked_align_up(layout.size, layout.alignment)?;
	if size == 0 {
		return Err(
			"Zero storage-buffer stride. The most likely cause is that the array element has no storage representation."
				.to_string(),
		);
	}
	u32::try_from(size).map_err(|_| {
		"Storage-buffer stride exceeds u32. The most likely cause is that an array element is excessively large.".to_string()
	})
}

/// Reflects one storage-buffer element using the selected backend's emitted layout.
pub(super) fn reflected_storage_buffer_stride_for_target(
	members: &[besl::NodeReference],
	target: StorageLayoutTarget,
) -> Result<u32, String> {
	if members.is_empty() {
		return Err(
			"Empty storage-buffer layout. The most likely cause is that the binding type declares no addressable members."
				.to_string(),
		);
	}

	// The lexer lowers a lone fixed-array member to an array buffer, so these buffers are always wrapper structs.
	let size = reflected_storage_members_layout(members, target, &mut HashSet::new())?.size;

	if size == 0 {
		return Err(
			"Zero storage-buffer stride. The most likely cause is that the binding contains a type without a storage representation."
				.to_string(),
		);
	}
	u32::try_from(size).map_err(|_| {
		"Storage-buffer stride exceeds u32. The most likely cause is that a reflected element contains an excessively large fixed array."
			.to_string()
	})
}

/// Computes the aligned layout of all members in one emitted storage struct.
fn reflected_storage_members_layout(
	members: &[besl::NodeReference],
	target: StorageLayoutTarget,
	visiting: &mut HashSet<besl::NodeReference>,
) -> Result<StorageLayout, String> {
	let mut size = 0usize;
	let mut alignment = 1usize;
	for member in members {
		let member = member.borrow();
		let besl::Nodes::Member { name, r#type, count } = member.node() else {
			return Err(
				"Unsupported storage-buffer member. The most likely cause is that a buffer layout contains a node other than a named member."
					.to_string(),
			);
		};
		let element = reflected_storage_type_layout(r#type, target, visiting)?;
		let member_alignment = element.alignment;
		let element_stride = checked_align_up(element.size, member_alignment)?;
		let count = count.map(std::num::NonZeroUsize::get).unwrap_or(1);
		let member_size = element_stride.checked_mul(count).ok_or_else(|| {
			format!(
				"Storage-buffer member '{name}' is too large. The most likely cause is that its fixed array count overflows the reflected layout."
			)
		})?;
		size = checked_align_up(size, member_alignment)?;
		size = size.checked_add(member_size).ok_or_else(|| {
			format!(
				"Storage-buffer layout overflows at member '{name}'. The most likely cause is that the reflected members exceed addressable memory."
			)
		})?;
		alignment = alignment.max(member_alignment);
	}
	Ok(StorageLayout {
		size: checked_align_up(size, alignment)?,
		alignment,
	})
}

/// Returns the emitted storage layout for one BESL value type.
pub(super) fn reflected_storage_type_layout(
	r#type: &besl::NodeReference,
	target: StorageLayoutTarget,
	visiting: &mut HashSet<besl::NodeReference>,
) -> Result<StorageLayout, String> {
	let type_borrow = r#type.borrow();
	let type_name = type_borrow.get_name().unwrap_or("unknown");
	if let Some(layout) = primitive_storage_layout(type_name, target) {
		return Ok(layout);
	}

	let fields = match type_borrow.node() {
		besl::Nodes::Struct { fields, .. } if !fields.is_empty() => fields.clone(),
		_ => {
			return Err(format!(
				"Unsupported storage-buffer type '{type_name}'. The most likely cause is that the binding contains a resource handle or a type without a packed storage representation."
			));
		}
	};
	let type_name = type_name.to_string();
	drop(type_borrow);

	if !visiting.insert(r#type.clone()) {
		return Err(format!(
			"Recursive storage-buffer type '{type_name}'. The most likely cause is that a shader struct contains itself."
		));
	}
	let layout = reflected_storage_members_layout(&fields, target, visiting);
	visiting.remove(r#type);
	layout
}

/// Returns the backend layout for one built-in BESL storage type.
pub(super) fn primitive_storage_layout(type_name: &str, target: StorageLayoutTarget) -> Option<StorageLayout> {
	let (size, alignment) = match (target, type_name) {
		// BESL u8 remains a 32-bit uint in HLSL.
		(StorageLayoutTarget::Hlsl, "u8") => (4, 4),
		// Metal stores a one-byte bool and keeps its native column alignment for matrices other than `mat4x3f`.
		(StorageLayoutTarget::Msl, "bool") => (1, 1),
		(StorageLayoutTarget::Msl, "mat2f") => (16, 8),
		(StorageLayoutTarget::Msl, "mat3f") => (48, 16),
		(StorageLayoutTarget::Msl, "mat4f") => (64, 16),
		// Every other type has one layout on every backend. Metal stores vectors as packed types, and native u16 values
		// keep their two-byte object representation and scalar alignment.
		(_, "u8") => (1, 1),
		(_, "u16" | "f16") => (2, 2),
		(_, "bool" | "u32" | "atomicu32" | "i32" | "atomici32" | "f32") => (4, 4),
		(_, "vec2u16" | "vec2f16") => (4, 2),
		(_, "vec3f16") => (6, 2),
		(_, "vec4u16" | "vec4f16") => (8, 2),
		(_, "vec2i" | "vec2u" | "vec2f") => (8, 4),
		(_, "vec3u" | "vec3f") => (12, 4),
		(_, "vec4u" | "vec4f") => (16, 4),
		(_, "mat2f") => (16, 4),
		(_, "mat3f") => (36, 4),
		(_, "mat4f") => (64, 4),
		// Metal expressions use a native float4x3, but buffer storage lowers to four packed_float3 columns.
		(_, "mat4x3f") => (48, 4),
		_ => return None,
	};
	Some(StorageLayout { size, alignment })
}

/// Rounds a reflected byte offset up without allowing arithmetic overflow.
pub(super) fn checked_align_up(value: usize, alignment: usize) -> Result<usize, String> {
	let remainder = value % alignment;
	if remainder == 0 {
		return Ok(value);
	}
	value.checked_add(alignment - remainder).ok_or_else(|| {
		"Storage-buffer alignment overflow. The most likely cause is that the reflected layout exceeds addressable memory."
			.to_string()
	})
}

use super::opacity::{OpacityEvaluation, evaluate_opacity};

/// The `ProgramEvaluation` struct holds information derived from evaluating a BESL program.
#[derive(Clone, Debug)]
/// The `ProgramEvaluation` struct holds binding reflection and output opacity for one BESL program.
pub struct ProgramEvaluation {
	bindings: Vec<BindingUsage>,
	opacity: OpacityEvaluation,
}

impl ProgramEvaluation {
	/// Reflects every declared binding while evaluating code behavior from reachable `main`.
	pub fn from_program(program: &besl::NodeReference) -> Result<Self, String> {
		let main = program.get_main().ok_or_else(|| {
			"Main function not found. The program description likely does not define a `main` function.".to_string()
		})?;

		besl::optimization::optimize(&main);

		Ok(Self {
			bindings: collect_bindings(program)?,
			opacity: evaluate_opacity(&main),
		})
	}

	pub fn from_main(main_function_node: &besl::NodeReference) -> Result<Self, String> {
		{
			let node_borrow = RefCell::borrow(main_function_node);
			let node_ref = node_borrow.node();

			match node_ref {
				besl::Nodes::Function { name, .. } => {
					if name != "main" {
						return Err(
							"Main node is not `main`. The program description likely passed a non-main function node."
								.to_string(),
						);
					}
				}
				_ => {
					return Err(
						"Invalid main node. The program description likely contains a `main` symbol that is not a function."
							.to_string(),
					);
				}
			}
		}

		besl::optimization::optimize(main_function_node);

		let bindings = collect_bindings(main_function_node)?;

		let opacity = evaluate_opacity(main_function_node);

		Ok(Self { bindings, opacity })
	}

	pub fn bindings(&self) -> &[BindingUsage] {
		&self.bindings
	}

	pub fn into_bindings(self) -> Vec<BindingUsage> {
		self.bindings
	}

	pub fn opacity(&self) -> OpacityEvaluation {
		self.opacity
	}
}

/// Collects sorted binding metadata while sharing repeated references and rejecting distinct slot aliases.
pub(crate) fn collect_bindings(node: &besl::NodeReference) -> Result<Vec<BindingUsage>, String> {
	let mut bindings = Vec::with_capacity(16);
	let mut state = BindingCollectionState {
		visited: utils::hash::HashSet::default(),
		error: None,
	};
	build_bindings(&mut bindings, node, &mut state);
	if let Some(error) = state.error {
		return Err(error);
	}

	bindings.sort_by_key(|binding| binding.slot);
	for (index, binding) in bindings.iter().enumerate() {
		let (slot, count) = (binding.slot, binding.count);
		let end_slot = slot.checked_add(count).ok_or_else(|| {
			format!(
				"Resource slot range overflow at slot {slot}. The most likely cause is that the declared resource range has no representable exclusive end."
			)
		})?;
		if let Some(next) = bindings.get(index + 1) {
			let next_slot = next.slot;
			if next_slot < end_slot {
				return Err(format!(
					"Resource slot ranges overlap at slots {slot} and {next_slot}. The most likely cause is that a resource array reserves a slot used by another declaration."
				));
			}
		}
	}

	Ok(bindings)
}

// This is the exhaustive BESL-node traversal contract for reflection; splitting it would duplicate child-edge rules.
#[allow(clippy::too_many_lines)]
fn build_bindings(bindings: &mut Vec<BindingUsage>, node: &besl::NodeReference, state: &mut BindingCollectionState) {
	if state.error.is_some() || !state.visited.insert(node.clone()) {
		return;
	}
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => {
			for statement in statements {
				build_bindings(bindings, statement, state);
			}
		}
		branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => {
			for child in branch.branch_children() {
				build_bindings(bindings, child, state);
			}
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			build_bindings(bindings, initializer, state);
			build_bindings(bindings, condition, state);
			build_bindings(bindings, update, state);
			for statement in statements {
				build_bindings(bindings, statement, state);
			}
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			} => {
				build_bindings(bindings, &callable.get(), state);
				for argument in arguments {
					build_bindings(bindings, argument, state);
				}
			}
			besl::Expressions::IntrinsicCall { arguments, elements, .. } => {
				// Intrinsic lowering emits the instantiated elements, not the definition template.
				for element in arguments.iter().chain(elements) {
					build_bindings(bindings, element, state);
				}
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				build_bindings(bindings, left, state);
				build_bindings(bindings, right, state);
			}
			besl::Expressions::Unary { operand, .. } => build_bindings(bindings, operand, state),
			besl::Expressions::Ternary {
				condition,
				if_true,
				if_false,
			} => {
				for part in [condition, if_true, if_false] {
					build_bindings(bindings, part, state);
				}
			}
			besl::Expressions::Expression { elements } => {
				for element in elements {
					build_bindings(bindings, element, state);
				}
			}
			besl::Expressions::Macro { body, .. } => {
				build_bindings(bindings, body, state);
			}
			besl::Expressions::Member { source, .. } => {
				build_bindings(bindings, source, state);
			}
			besl::Expressions::VariableDeclaration { r#type, .. } => {
				build_bindings(bindings, r#type, state);
			}
			besl::Expressions::Return { value } => {
				// A returned expression can be the only path from main to a resource used by a helper function.
				if let Some(value) = value {
					build_bindings(bindings, value, state);
				}
			}
			besl::Expressions::Literal { .. }
			| besl::Expressions::Continue
			| besl::Expressions::Break
			| besl::Expressions::Discard => {}
		},
		besl::Nodes::Binding {
			name,
			slot,
			read,
			write,
			r#type,
			count,
			..
		} => {
			let (kind, buffer_stride) = match r#type {
				besl::BindingTypes::Buffer { members } => {
					let stride = match reflected_storage_buffer_stride(members) {
						Ok(stride) => stride,
						Err(error) => {
							state.error = Some(format!("Failed to reflect storage-buffer binding '{name}'. {error}"));
							return;
						}
					};
					(BindingKind::StorageBuffer, Some(stride))
				}
				besl::BindingTypes::BufferArray { element, .. } => {
					let stride = match reflected_array_buffer_stride(element) {
						Ok(stride) => stride,
						Err(error) => {
							state.error = Some(format!("Failed to reflect storage-buffer binding '{name}'. {error}"));
							return;
						}
					};
					(BindingKind::StorageBuffer, Some(stride))
				}
				besl::BindingTypes::CombinedImageSampler { format } => (
					BindingKind::CombinedImageSampler {
						view: match format.as_str() {
							"Texture3D" => TextureView::Texture3D,
							"TextureCube" => TextureView::TextureCube,
							"TextureCubeArray" => TextureView::TextureCubeArray,
							"ArrayTexture2D" => TextureView::Texture2DArray,
							_ => TextureView::Texture2D,
						},
					},
					None,
				),
				besl::BindingTypes::Image { .. } => (BindingKind::StorageImage, None),
			};
			let count = count.map_or(1, |count| count.get());
			if bindings.iter().any(|record| record.slot == *slot) {
				state.error = Some(format!(
					"Duplicate resource declaration at slot {slot}. The most likely cause is that distinct binding nodes reuse one flat slot instead of sharing the same binding reference."
				));
			} else {
				bindings.push(BindingUsage {
					name: name.to_string(),
					kind,
					count,
					slot: *slot,
					buffer_stride,
					read: *read,
					write: *write,
				});
			}
		}
		besl::Nodes::Raw { input, output, .. } => {
			for reference in input.iter().chain(output.iter()) {
				build_bindings(bindings, reference, state);
			}
		}
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			for element in elements {
				build_bindings(bindings, element, state);
			}
			build_bindings(bindings, r#return, state);
		}
		besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			build_bindings(bindings, nested, state);
		}
		besl::Nodes::Input { format, .. }
		| besl::Nodes::Output { format, .. }
		| besl::Nodes::TaskPayload { format, .. }
		| besl::Nodes::Workgroup { format, .. } => {
			build_bindings(bindings, format, state);
		}
		besl::Nodes::PushConstant { members: nested } => {
			if let Err(error) = super::push_constant_layout::validate_push_constant_layout(nested) {
				state.error = Some(error);
				return;
			}
			for child in nested {
				build_bindings(bindings, child, state);
			}
		}
		besl::Nodes::Struct { fields: nested, .. } | besl::Nodes::Scope { children: nested, .. } => {
			for child in nested {
				build_bindings(bindings, child, state);
			}
		}
		besl::Nodes::Const { r#type, value, .. } => {
			build_bindings(bindings, r#type, state);
			build_bindings(bindings, value, state);
		}
	}
}
