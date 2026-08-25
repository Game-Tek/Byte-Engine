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

/// The `BindingRecord` trait keeps binding discovery independent of evaluated and compiled metadata representations.
pub(crate) trait BindingRecord: Sized {
	fn from_usage(
		name: &str,
		kind: BindingKind,
		count: u32,
		slot: u32,
		buffer_stride: Option<u32>,
		read: bool,
		write: bool,
	) -> Self;
	fn usage(&self) -> (u32, BindingKind, u32, bool, bool);
}

impl BindingRecord for BindingUsage {
	fn from_usage(
		name: &str,
		kind: BindingKind,
		count: u32,
		slot: u32,
		buffer_stride: Option<u32>,
		read: bool,
		write: bool,
	) -> Self {
		Self {
			name: name.to_string(),
			kind,
			count,
			slot,
			buffer_stride,
			read,
			write,
		}
	}

	fn usage(&self) -> (u32, BindingKind, u32, bool, bool) {
		(self.slot, self.kind, self.count, self.read, self.write)
	}
}

/// The `BindingCollectionState` struct keeps reflection traversal aligned with graph identity deduplication.
struct BindingCollectionState {
	visited: HashSet<besl::NodeReference>,
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

	let mut visiting = HashSet::new();
	// A single array member is lowered as a linear element buffer. Other
	// layouts retain their wrapper struct and therefore use the wrapper size.
	let size = if let [member] = members {
		let member = member.borrow();
		match member.node() {
			besl::Nodes::Member {
				r#type, count: Some(_), ..
			} => {
				let element = reflected_storage_member_type_layout(r#type, target, true, true, &mut visiting)?;
				checked_align_up(element.size, element.alignment)?
			}
			_ => reflected_storage_members_layout(members, target, true, &mut visiting)?.size,
		}
	} else {
		reflected_storage_members_layout(members, target, true, &mut visiting)?.size
	};

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
	direct_binding_members: bool,
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
		let element = reflected_storage_member_type_layout(r#type, target, direct_binding_members, count.is_some(), visiting)?;
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

/// Applies direct Metal buffer-member packing before reflecting the member type.
fn reflected_storage_member_type_layout(
	r#type: &besl::NodeReference,
	target: StorageLayoutTarget,
	direct_binding_member: bool,
	array_member: bool,
	visiting: &mut HashSet<besl::NodeReference>,
) -> Result<StorageLayout, String> {
	let packed_msl_vector = target == StorageLayoutTarget::Msl
		&& direct_binding_member
		&& (array_member
			|| matches!(
				r#type.borrow().get_name(),
				Some("vec2f16" | "vec3f16" | "vec4f16" | "vec2u16" | "vec4u16")
			));
	reflected_storage_type_layout(r#type, target, packed_msl_vector, visiting)
}

/// Returns the native emitted storage layout for one BESL value type.
pub(super) fn reflected_storage_type_layout(
	r#type: &besl::NodeReference,
	target: StorageLayoutTarget,
	packed_msl_vector: bool,
	visiting: &mut HashSet<besl::NodeReference>,
) -> Result<StorageLayout, String> {
	let type_borrow = r#type.borrow();
	let type_name = type_borrow.get_name().unwrap_or("unknown");
	if let Some(layout) = primitive_storage_layout(type_name, target, packed_msl_vector) {
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
	// Nested Metal structs use their native member types. Only members written
	// directly into a generated binding wrapper receive packed vector aliases.
	let layout = reflected_storage_members_layout(&fields, target, false, visiting);
	visiting.remove(r#type);
	layout
}

/// Returns the backend layout for one built-in BESL storage type.
pub(super) fn primitive_storage_layout(
	type_name: &str,
	target: StorageLayoutTarget,
	packed_msl_vector: bool,
) -> Option<StorageLayout> {
	let (size, alignment) = match target {
		StorageLayoutTarget::Hlsl => match type_name {
			// HLSL lowers narrow integer scalar values to 32-bit uint values. Its
			// structured-buffer vectors and row-major matrices use scalar alignment.
			"bool" | "u8" | "u16" | "u32" | "atomicu32" | "i32" | "f32" => (4, 4),
			"f16" => (2, 2),
			"vec2u16" => (4, 2),
			"vec4u16" => (8, 2),
			"vec2f16" => (4, 2),
			"vec3f16" => (6, 2),
			"vec4f16" => (8, 2),
			"vec2i" | "vec2u" | "vec2f" => (8, 4),
			"vec3u" | "vec3f" => (12, 4),
			"vec4u" | "vec4f" | "packed_vec4f" => (16, 4),
			"mat2f" => (16, 4),
			"mat3f" => (36, 4),
			"mat4f" => (64, 4),
			"mat4x3f" => (48, 4),
			_ => return None,
		},
		StorageLayoutTarget::Msl => match type_name {
			"bool" | "u8" => (1, 1),
			"u16" | "f16" => (2, 2),
			"u32" | "atomicu32" | "i32" | "f32" => (4, 4),
			"vec2u16" => (4, if packed_msl_vector { 2 } else { 4 }),
			"vec4u16" => (8, if packed_msl_vector { 2 } else { 8 }),
			"vec2f16" => (4, if packed_msl_vector { 2 } else { 4 }),
			"vec3f16" => {
				if packed_msl_vector {
					(6, 2)
				} else {
					(8, 8)
				}
			}
			"vec4f16" => (8, if packed_msl_vector { 2 } else { 8 }),
			"vec2f" => (8, if packed_msl_vector { 4 } else { 8 }),
			"vec2i" | "vec2u" => (8, 8),
			"vec3f" => {
				if packed_msl_vector {
					(12, 4)
				} else {
					(16, 16)
				}
			}
			"vec3u" => (16, 16),
			"vec4u" | "vec4f" => (16, 16),
			// Explicit packed vectors retain scalar alignment inside nested records.
			"packed_vec4f" => (16, 4),
			"mat2f" => (16, 8),
			"mat3f" => (48, 16),
			"mat4f" => (64, 16),
			// MSL expressions use native float4x3, but buffer storage lowers to four packed_float3 columns.
			"mat4x3f" => (48, 4),
			_ => return None,
		},
		StorageLayoutTarget::GlslScalar => match type_name {
			"u8" => (1, 1),
			"u16" | "f16" => (2, 2),
			"bool" | "u32" | "atomicu32" | "i32" | "f32" => (4, 4),
			"vec2u16" => (4, 2),
			"vec4u16" => (8, 2),
			"vec2f16" => (4, 2),
			"vec3f16" => (6, 2),
			"vec4f16" => (8, 2),
			"vec2i" | "vec2u" | "vec2f" => (8, 4),
			"vec3u" | "vec3f" => (12, 4),
			"vec4u" | "vec4f" | "packed_vec4f" => (16, 4),
			"mat2f" => (16, 4),
			"mat3f" => (36, 4),
			"mat4f" => (64, 4),
			"mat4x3f" => (48, 4),
			_ => return None,
		},
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
pub(crate) fn collect_bindings<T: BindingRecord>(node: &besl::NodeReference) -> Result<Vec<T>, String> {
	let mut bindings: Vec<T> = Vec::with_capacity(16);
	let mut state = BindingCollectionState {
		visited: HashSet::new(),
		error: None,
	};
	build_bindings(&mut bindings, node, &mut state);
	if let Some(error) = state.error {
		return Err(error);
	}

	bindings.sort_by_key(|binding| binding.usage().0);
	for (index, binding) in bindings.iter().enumerate() {
		let (slot, _, count, ..) = binding.usage();
		let end_slot = slot.checked_add(count).ok_or_else(|| {
			format!(
				"Resource slot range overflow at slot {slot}. The most likely cause is that the declared resource range has no representable exclusive end."
			)
		})?;
		if let Some(next) = bindings.get(index + 1) {
			let (next_slot, ..) = next.usage();
			if next_slot < end_slot {
				return Err(format!(
					"Resource slot ranges overlap at slots {slot} and {next_slot}. The most likely cause is that a resource array reserves a slot used by another declaration."
				));
			}
		}
	}

	Ok(bindings)
}

fn build_bindings<T: BindingRecord>(bindings: &mut Vec<T>, node: &besl::NodeReference, state: &mut BindingCollectionState) {
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
		besl::Nodes::Conditional { condition, statements } => {
			build_bindings(bindings, condition, state);
			for statement in statements {
				build_bindings(bindings, statement, state);
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
				build_bindings(bindings, callable, state);
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
			besl::Expressions::Literal { .. } | besl::Expressions::Continue | besl::Expressions::Discard => {}
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
			if bindings.iter().any(|record| record.usage().0 == *slot) {
				state.error = Some(format!(
					"Duplicate resource declaration at slot {slot}. The most likely cause is that distinct binding nodes reuse one flat slot instead of sharing the same binding reference."
				));
			} else {
				bindings.push(T::from_usage(name, kind, count, *slot, buffer_stride, *read, *write));
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
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
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
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => {
			for child in nested {
				build_bindings(bindings, child, state);
			}
		}
		besl::Nodes::Null => {}
		besl::Nodes::Const { r#type, value, .. } => {
			build_bindings(bindings, r#type, state);
			build_bindings(bindings, value, state);
		}
	}
}
