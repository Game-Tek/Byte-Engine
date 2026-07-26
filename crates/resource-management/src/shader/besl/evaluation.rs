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
enum StorageLayoutTarget {
	Hlsl,
	Msl,
	GlslScalar,
}

impl StorageLayoutTarget {
	/// Selects the layout model that matches the backend compiled for this target.
	const fn current() -> Self {
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
struct StorageLayout {
	size: usize,
	alignment: usize,
}

/// Reflects the byte stride used when one storage-buffer element is addressed.
fn reflected_storage_buffer_stride(members: &[besl::NodeReference]) -> Result<u32, String> {
	reflected_storage_buffer_stride_for_target(members, StorageLayoutTarget::current())
}

/// Reflects one storage-buffer element using the selected backend's emitted layout.
fn reflected_storage_buffer_stride_for_target(
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
		&& (array_member || matches!(r#type.borrow().get_name(), Some("vec2u16" | "vec4u16")));
	reflected_storage_type_layout(r#type, target, packed_msl_vector, visiting)
}

/// Returns the native emitted storage layout for one BESL value type.
fn reflected_storage_type_layout(
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
fn primitive_storage_layout(type_name: &str, target: StorageLayoutTarget, packed_msl_vector: bool) -> Option<StorageLayout> {
	let (size, alignment) = match target {
		StorageLayoutTarget::Hlsl => match type_name {
			// HLSL lowers narrow scalar values to 32-bit uint values. Its
			// structured-buffer vectors and row-major matrices use scalar alignment.
			"bool" | "u8" | "u16" | "u32" | "atomicu32" | "i32" | "f32" => (4, 4),
			"vec2u16" => (4, 2),
			"vec4u16" => (8, 2),
			"vec2i" | "vec2u" | "vec2f" => (8, 4),
			"vec3u" | "vec3f" => (12, 4),
			"vec4u" | "vec4f" => (16, 4),
			"mat2f" => (16, 4),
			"mat3f" => (36, 4),
			"mat4f" => (64, 4),
			"mat4x3f" => (48, 4),
			_ => return None,
		},
		StorageLayoutTarget::Msl => match type_name {
			"bool" | "u8" => (1, 1),
			"u16" => (2, 2),
			"u32" | "atomicu32" | "i32" | "f32" => (4, 4),
			"vec2u16" => (4, if packed_msl_vector { 2 } else { 4 }),
			"vec4u16" => (8, if packed_msl_vector { 2 } else { 8 }),
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
			"mat2f" => (16, 8),
			"mat3f" => (48, 16),
			"mat4f" | "mat4x3f" => (64, 16),
			_ => return None,
		},
		StorageLayoutTarget::GlslScalar => match type_name {
			"u8" => (1, 1),
			"u16" => (2, 2),
			"bool" | "u32" | "atomicu32" | "i32" | "f32" => (4, 4),
			"vec2u16" => (4, 2),
			"vec4u16" => (8, 2),
			"vec2i" | "vec2u" | "vec2f" => (8, 4),
			"vec3u" | "vec3f" => (12, 4),
			"vec4u" | "vec4f" => (16, 4),
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
fn checked_align_up(value: usize, alignment: usize) -> Result<usize, String> {
	let remainder = value % alignment;
	if remainder == 0 {
		return Ok(value);
	}
	value.checked_add(alignment - remainder).ok_or_else(|| {
		"Storage-buffer alignment overflow. The most likely cause is that the reflected layout exceeds addressable memory."
			.to_string()
	})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpacityEvaluation {
	Opaque,
	NonOpaque,
	Unknown,
}

/// The `ProgramEvaluation` struct holds information derived from evaluating a BESL program.
#[derive(Clone, Debug)]
pub struct ProgramEvaluation {
	bindings: Vec<BindingUsage>,
	opacity: OpacityEvaluation,
}

impl ProgramEvaluation {
	pub fn from_program(program: &besl::NodeReference) -> Result<Self, String> {
		let main = program.get_main().ok_or_else(|| {
			"Main function not found. The program description likely does not define a `main` function.".to_string()
		})?;

		Self::from_main(&main)
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
			besl::Expressions::IntrinsicCall { elements, .. } => {
				// Intrinsic lowering emits the instantiated elements, not the definition template.
				for element in elements {
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
			besl::Expressions::Return { .. } | besl::Expressions::Literal { .. } | besl::Expressions::Continue => {}
		},
		besl::Nodes::Binding {
			name,
			slot,
			read,
			write,
			r#type,
			count,
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

fn evaluate_opacity(main_function_node: &besl::NodeReference) -> OpacityEvaluation {
	let mut main_contains_raw_code = false;
	let mut local_output_symbols = HashSet::new();

	{
		let node_borrow = RefCell::borrow(main_function_node);
		let node_ref = node_borrow.node();

		if let besl::Nodes::Function { statements, params, .. } = node_ref {
			for param in params {
				let param_borrow = RefCell::borrow(param);
				if let besl::Nodes::Parameter {
					name: parameter_name, ..
				} = param_borrow.node()
				{
					if parameter_name == "output" {
						local_output_symbols.insert(param.clone());
					}
				}
			}

			for statement in statements {
				let statement_borrow = RefCell::borrow(statement);
				match statement_borrow.node() {
					besl::Nodes::Raw { .. } => {
						main_contains_raw_code = true;
					}
					_ => collect_local_output_symbols(statement, &mut local_output_symbols),
				}
			}
		}
	}

	if main_contains_raw_code {
		return OpacityEvaluation::Unknown;
	}

	if writes_non_opaque_vec4f_to_non_local_output(main_function_node, &local_output_symbols) {
		return OpacityEvaluation::NonOpaque;
	}

	if references_non_local_output(main_function_node, &local_output_symbols) {
		OpacityEvaluation::Opaque
	} else {
		OpacityEvaluation::Unknown
	}
}

fn collect_local_output_symbols(node: &besl::NodeReference, local_output_symbols: &mut HashSet<besl::NodeReference>) {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, params, .. } => {
			for param in params {
				collect_local_output_symbols(param, local_output_symbols);
			}
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::Conditional { condition, statements } => {
			collect_local_output_symbols(condition, local_output_symbols);
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			collect_local_output_symbols(initializer, local_output_symbols);
			collect_local_output_symbols(condition, local_output_symbols);
			collect_local_output_symbols(update, local_output_symbols);
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::VariableDeclaration { name, .. } => {
				if name == "output" {
					local_output_symbols.insert(node.clone());
				}
			}
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				collect_local_output_symbols(callable, local_output_symbols);
				for argument in arguments {
					collect_local_output_symbols(argument, local_output_symbols);
				}
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				collect_local_output_symbols(left, local_output_symbols);
				collect_local_output_symbols(right, local_output_symbols);
			}
			besl::Expressions::Expression { elements } => {
				for element in elements {
					collect_local_output_symbols(element, local_output_symbols);
				}
			}
			besl::Expressions::Member { source, .. } => {
				collect_local_output_symbols(source, local_output_symbols);
			}
			besl::Expressions::Macro { body, .. } => {
				collect_local_output_symbols(body, local_output_symbols);
			}
			besl::Expressions::Return { .. } | besl::Expressions::Literal { .. } | besl::Expressions::Continue => {}
		},
		besl::Nodes::Raw { input, output, .. } => {
			for value in input.iter().chain(output.iter()) {
				collect_local_output_symbols(value, local_output_symbols);
			}
		}
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			for element in elements {
				collect_local_output_symbols(element, local_output_symbols);
			}
			collect_local_output_symbols(r#return, local_output_symbols);
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			collect_local_output_symbols(nested, local_output_symbols);
		}
		besl::Nodes::Parameter {
			name: parameter_name,
			r#type: parameter_type,
		} => {
			if parameter_name == "output" {
				local_output_symbols.insert(node.clone());
			}
			collect_local_output_symbols(parameter_type, local_output_symbols);
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => {
			for child in nested {
				collect_local_output_symbols(child, local_output_symbols);
			}
		}
		besl::Nodes::Binding { .. } | besl::Nodes::Null => {}
		besl::Nodes::Const { r#type, value, .. } => {
			collect_local_output_symbols(r#type, local_output_symbols);
			collect_local_output_symbols(value, local_output_symbols);
		}
	}
}

fn references_non_local_output(node: &besl::NodeReference, local_output_symbols: &HashSet<besl::NodeReference>) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => statements
			.iter()
			.any(|statement| references_non_local_output(statement, local_output_symbols)),
		besl::Nodes::Conditional { condition, statements } => {
			references_non_local_output(condition, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| references_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			references_non_local_output(initializer, local_output_symbols)
				|| references_non_local_output(condition, local_output_symbols)
				|| references_non_local_output(update, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| references_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Member { name, source } => {
				if name == "output" && !local_output_symbols.contains(source) {
					return true;
				}

				references_non_local_output(source, local_output_symbols)
			}
			besl::Expressions::Expression { elements } => elements
				.iter()
				.any(|element| references_non_local_output(element, local_output_symbols)),
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				references_non_local_output(callable, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| references_non_local_output(argument, local_output_symbols))
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				references_non_local_output(left, local_output_symbols)
					|| references_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::VariableDeclaration { r#type: nested, .. } | besl::Expressions::Macro { body: nested, .. } => {
				references_non_local_output(nested, local_output_symbols)
			}
			besl::Expressions::Return { .. } | besl::Expressions::Literal { .. } | besl::Expressions::Continue => false,
		},
		besl::Nodes::Raw { input, output, .. } => input
			.iter()
			.chain(output.iter())
			.any(|reference| references_non_local_output(reference, local_output_symbols)),
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			elements
				.iter()
				.any(|element| references_non_local_output(element, local_output_symbols))
				|| references_non_local_output(r#return, local_output_symbols)
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => references_non_local_output(nested, local_output_symbols),
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => nested
			.iter()
			.any(|child| references_non_local_output(child, local_output_symbols)),
		besl::Nodes::Binding { .. } | besl::Nodes::Null => false,
		besl::Nodes::Const { r#type, value, .. } => {
			references_non_local_output(r#type, local_output_symbols)
				|| references_non_local_output(value, local_output_symbols)
		}
	}
}

fn writes_non_opaque_vec4f_to_non_local_output(
	node: &besl::NodeReference,
	local_output_symbols: &HashSet<besl::NodeReference>,
) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => statements
			.iter()
			.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols)),
		besl::Nodes::Conditional { condition, statements } => {
			writes_non_opaque_vec4f_to_non_local_output(condition, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			writes_non_opaque_vec4f_to_non_local_output(initializer, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(condition, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(update, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Operator { operator, left, right } => {
				if operator == &besl::Operators::Assignment
					&& is_non_local_output_target(left, local_output_symbols)
					&& is_non_opaque_vec4f_constructor(right)
				{
					return true;
				}

				writes_non_opaque_vec4f_to_non_local_output(left, local_output_symbols)
					|| writes_non_opaque_vec4f_to_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::Expression { elements } => elements
				.iter()
				.any(|element| writes_non_opaque_vec4f_to_non_local_output(element, local_output_symbols)),
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				writes_non_opaque_vec4f_to_non_local_output(callable, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| writes_non_opaque_vec4f_to_non_local_output(argument, local_output_symbols))
			}
			besl::Expressions::Accessor { left, right } => {
				writes_non_opaque_vec4f_to_non_local_output(left, local_output_symbols)
					|| writes_non_opaque_vec4f_to_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::Member { source, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(source, local_output_symbols)
			}
			besl::Expressions::VariableDeclaration { r#type: nested, .. } | besl::Expressions::Macro { body: nested, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(nested, local_output_symbols)
			}
			besl::Expressions::Return { .. } | besl::Expressions::Literal { .. } | besl::Expressions::Continue => false,
		},
		besl::Nodes::Raw { input, output, .. } => input
			.iter()
			.chain(output.iter())
			.any(|reference| writes_non_opaque_vec4f_to_non_local_output(reference, local_output_symbols)),
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			elements
				.iter()
				.any(|element| writes_non_opaque_vec4f_to_non_local_output(element, local_output_symbols))
				|| writes_non_opaque_vec4f_to_non_local_output(r#return, local_output_symbols)
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			writes_non_opaque_vec4f_to_non_local_output(nested, local_output_symbols)
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => nested
			.iter()
			.any(|child| writes_non_opaque_vec4f_to_non_local_output(child, local_output_symbols)),
		besl::Nodes::Binding { .. } | besl::Nodes::Null => false,
		besl::Nodes::Const { r#type, value, .. } => {
			writes_non_opaque_vec4f_to_non_local_output(r#type, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(value, local_output_symbols)
		}
	}
}

fn is_non_local_output_target(node: &besl::NodeReference, local_output_symbols: &HashSet<besl::NodeReference>) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::Member {
			name: member_name,
			source: member_source,
		}) => member_name == "output" && !local_output_symbols.contains(member_source),
		besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) => {
			is_non_local_output_target(left, local_output_symbols)
		}
		_ => false,
	}
}

fn is_non_opaque_vec4f_constructor(node: &besl::NodeReference) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			let function_borrow = RefCell::borrow(function);
			if function_borrow.get_name() != Some("vec4f") {
				return false;
			}

			let w_parameter = match parameters.len() {
				4 => Some(&parameters[3]),
				2 if is_vec3f_constructor(&parameters[0]) => Some(&parameters[1]),
				_ => None,
			};

			let Some(w_parameter) = w_parameter else {
				return false;
			};

			match parse_literal_number(w_parameter) {
				Some(w) => w != 1.0,
				None => false,
			}
		}
		_ => false,
	}
}

fn is_vec3f_constructor(node: &besl::NodeReference) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			let function_borrow = RefCell::borrow(function);
			function_borrow.get_name() == Some("vec3f") && parameters.len() == 3
		}
		_ => false,
	}
}

fn parse_literal_number(node: &besl::NodeReference) -> Option<f64> {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::Literal { value }) => value.parse().ok(),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shader::generator;

	fn assert_builtin_layout(root: &besl::Node, target: StorageLayoutTarget, type_name: &str, expected: StorageLayout) {
		let r#type = root
			.get_child(type_name)
			.unwrap_or_else(|| panic!("Expected built-in type '{type_name}'"));
		let layout = reflected_storage_type_layout(&r#type, target, false, &mut HashSet::new())
			.unwrap_or_else(|error| panic!("Expected '{type_name}' layout for {target:?}: {error}"));
		assert_eq!(layout, expected, "Unexpected '{type_name}' layout for {target:?}");
	}

	#[test]
	fn binding_metadata_is_sorted_and_classified() {
		let main = generator::tests::bindings();

		let evaluation = ProgramEvaluation::from_main(&main).expect("Failed to evaluate program");
		let bindings = evaluation
			.bindings()
			.iter()
			.map(|binding| {
				(
					binding.name.as_str(),
					binding.kind,
					binding.count,
					binding.slot,
					binding.buffer_stride,
					binding.read,
					binding.write,
				)
			})
			.collect::<Vec<_>>();

		assert_eq!(
			bindings,
			vec![
				("buff", BindingKind::StorageBuffer, 1, 0, Some(4), true, true),
				("image", BindingKind::StorageImage, 1, 1, None, false, true),
				(
					"texture",
					BindingKind::CombinedImageSampler {
						view: TextureView::Texture2D,
					},
					1,
					2,
					None,
					true,
					false,
				),
			]
		);
	}

	#[test]
	fn storage_buffer_strides_cover_flattened_arrays_and_wrapper_structs() {
		let script = "main: fn () -> void { positions; indices; lighting; }";
		let mut root = besl::Node::root();
		let vec3f = root.get_child("vec3f").expect("Expected vec3f");
		let vec2f = root.get_child("vec2f").expect("Expected vec2f");
		let u8_type = root.get_child("u8").expect("Expected u8");
		let u16_type = root.get_child("u16").expect("Expected u16");
		let u32_type = root.get_child("u32").expect("Expected u32");
		let light = root.add_child(
			besl::Node::r#struct(
				"Light",
				vec![
					besl::Node::member("position", vec3f.clone()).into(),
					besl::Node::member("color", vec3f.clone()).into(),
					besl::Node::member("direction", vec3f.clone()).into(),
					besl::Node::member("cone_cosines", vec2f).into(),
					besl::Node::member("light_type", u8_type).into(),
					besl::Node::array("cascades", u32_type.clone(), 8),
				],
			)
			.into(),
		);
		root.add_children(vec![
			besl::Node::binding(
				"positions",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("positions", vec3f, 16)],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"indices",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("indices", u16_type, 16)],
				},
				1,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"lighting",
				besl::BindingTypes::Buffer {
					members: vec![
						besl::Node::member("light_count", u32_type).into(),
						besl::Node::array("lights", light, 16),
					],
				},
				2,
				true,
				false,
			)
			.into(),
		]);

		let program = besl::compile_to_besl(script, Some(root)).expect("Expected stride-reflection shader to link");
		let evaluation = ProgramEvaluation::from_program(&program).expect("Expected storage-buffer strides to reflect");
		let strides = evaluation
			.bindings()
			.iter()
			.map(|binding| binding.buffer_stride)
			.collect::<Vec<_>>();

		let expected = if cfg!(target_vendor = "apple") {
			vec![Some(12), Some(2), Some(1552)]
		} else if cfg!(target_os = "windows") {
			vec![Some(12), Some(4), Some(1284)]
		} else {
			vec![Some(12), Some(2), Some(1284)]
		};
		assert_eq!(strides, expected);
	}

	#[test]
	fn storage_layout_target_matches_the_compiled_backend() {
		#[cfg(target_vendor = "apple")]
		assert_eq!(StorageLayoutTarget::current(), StorageLayoutTarget::Msl);

		#[cfg(all(not(target_vendor = "apple"), target_os = "windows"))]
		assert_eq!(StorageLayoutTarget::current(), StorageLayoutTarget::Hlsl);

		#[cfg(all(not(target_vendor = "apple"), not(target_os = "windows")))]
		assert_eq!(StorageLayoutTarget::current(), StorageLayoutTarget::GlslScalar);
	}

	#[test]
	fn primitive_storage_layouts_follow_each_emitted_backend_type() {
		let root = besl::Node::root();

		for (type_name, size, alignment) in [
			("u8", 4, 4),
			("u16", 4, 4),
			("u32", 4, 4),
			("vec2u16", 4, 2),
			("vec4u16", 8, 2),
			("vec3f", 12, 4),
		] {
			assert_builtin_layout(&root, StorageLayoutTarget::Hlsl, type_name, StorageLayout { size, alignment });
		}

		for (type_name, size, alignment) in [
			("u8", 1, 1),
			("u16", 2, 2),
			("u32", 4, 4),
			("vec2u16", 4, 4),
			("vec4u16", 8, 8),
			("vec3f", 16, 16),
		] {
			assert_builtin_layout(&root, StorageLayoutTarget::Msl, type_name, StorageLayout { size, alignment });
		}

		for (type_name, size, alignment) in [
			("u8", 1, 1),
			("u16", 2, 2),
			("u32", 4, 4),
			("vec2u16", 4, 2),
			("vec4u16", 8, 2),
			("vec3f", 12, 4),
		] {
			assert_builtin_layout(
				&root,
				StorageLayoutTarget::GlslScalar,
				type_name,
				StorageLayout { size, alignment },
			);
		}
	}

	#[test]
	fn flattened_narrow_scalar_arrays_use_the_emitted_element_width() {
		let root = besl::Node::root();
		let u8_type = root.get_child("u8").expect("Expected u8");
		let u16_type = root.get_child("u16").expect("Expected u16");
		let bytes = vec![besl::Node::array("bytes", u8_type, 8)];
		let words = vec![besl::Node::array("words", u16_type, 8)];

		for (target, byte_stride, word_stride) in [
			(StorageLayoutTarget::Hlsl, 4, 4),
			(StorageLayoutTarget::Msl, 1, 2),
			(StorageLayoutTarget::GlslScalar, 1, 2),
		] {
			assert_eq!(reflected_storage_buffer_stride_for_target(&bytes, target), Ok(byte_stride));
			assert_eq!(reflected_storage_buffer_stride_for_target(&words, target), Ok(word_stride));
		}
	}

	#[test]
	fn matrix_storage_layouts_cover_all_besl_matrix_types() {
		let mut root = besl::Node::root();
		let vec2f = root.get_child("vec2f").expect("Expected vec2f");
		let vec3f = root.get_child("vec3f").expect("Expected vec3f");
		let mat2f = root.add_child(
			besl::Node::r#struct(
				"mat2f",
				vec![
					besl::Node::member("x", vec2f.clone()).into(),
					besl::Node::member("y", vec2f).into(),
				],
			)
			.into(),
		);
		let mat3f = root.add_child(
			besl::Node::r#struct(
				"mat3f",
				vec![
					besl::Node::member("x", vec3f.clone()).into(),
					besl::Node::member("y", vec3f.clone()).into(),
					besl::Node::member("z", vec3f).into(),
				],
			)
			.into(),
		);
		let matrices = [
			("mat2f", mat2f),
			("mat3f", mat3f),
			("mat4f", root.get_child("mat4f").expect("Expected mat4f")),
			("mat4x3f", root.get_child("mat4x3f").expect("Expected mat4x3f")),
		];

		for (target, expected) in [
			(StorageLayoutTarget::Hlsl, [(16, 4), (36, 4), (64, 4), (48, 4)]),
			(StorageLayoutTarget::Msl, [(16, 8), (48, 16), (64, 16), (64, 16)]),
			(StorageLayoutTarget::GlslScalar, [(16, 4), (36, 4), (64, 4), (48, 4)]),
		] {
			for ((type_name, matrix), (size, alignment)) in matrices.iter().zip(expected) {
				let layout = reflected_storage_type_layout(matrix, target, false, &mut HashSet::new())
					.unwrap_or_else(|error| panic!("Expected '{type_name}' layout for {target:?}: {error}"));
				assert_eq!(layout, StorageLayout { size, alignment });
			}
		}
	}

	#[test]
	fn nested_struct_arrays_apply_member_and_tail_alignment() {
		let mut root = besl::Node::root();
		let vec3f = root.get_child("vec3f").expect("Expected vec3f");
		let u8_type = root.get_child("u8").expect("Expected u8");
		let u32_type = root.get_child("u32").expect("Expected u32");
		let mixed = root.add_child(
			besl::Node::r#struct(
				"Mixed",
				vec![
					besl::Node::member("position", vec3f.clone()).into(),
					besl::Node::member("tag", u8_type).into(),
				],
			)
			.into(),
		);
		let wrapper = vec![
			besl::Node::array("items", mixed, 2),
			besl::Node::member("tail", u32_type).into(),
		];
		let scalar_position = vec![besl::Node::member("position", vec3f.clone()).into()];
		let flattened_positions = vec![besl::Node::array("positions", vec3f, 8)];

		assert_eq!(
			reflected_storage_buffer_stride_for_target(&wrapper, StorageLayoutTarget::Hlsl),
			Ok(36)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&wrapper, StorageLayoutTarget::Msl),
			Ok(80)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&wrapper, StorageLayoutTarget::GlslScalar),
			Ok(36)
		);

		// Metal emits packed_float3 only for the direct array member. Direct
		// scalar members and fields nested inside Mixed retain native float3.
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&scalar_position, StorageLayoutTarget::Hlsl),
			Ok(12)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&scalar_position, StorageLayoutTarget::Msl),
			Ok(16)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&scalar_position, StorageLayoutTarget::GlslScalar),
			Ok(12)
		);
		for target in [
			StorageLayoutTarget::Hlsl,
			StorageLayoutTarget::Msl,
			StorageLayoutTarget::GlslScalar,
		] {
			assert_eq!(
				reflected_storage_buffer_stride_for_target(&flattened_positions, target),
				Ok(12)
			);
		}
	}

	#[test]
	fn visibility_storage_structs_match_the_backend_abi() {
		let mut root = besl::Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32");
		let u32_type = root.get_child("u32").expect("Expected u32");
		let vec2f = root.get_child("vec2f").expect("Expected vec2f");
		let vec4f = root.get_child("vec4f").expect("Expected vec4f");
		let mat4f = root.get_child("mat4f").expect("Expected mat4f");
		let mat4x3f = root.get_child("mat4x3f").expect("Expected mat4x3f");

		let mesh = root.add_child(
			besl::Node::r#struct(
				"Mesh",
				vec![
					besl::Node::member("model", mat4x3f).into(),
					besl::Node::member("material_index", u32_type.clone()).into(),
					besl::Node::member("base_vertex_index", u32_type.clone()).into(),
					besl::Node::member("base_primitive_index", u32_type.clone()).into(),
					besl::Node::member("base_triangle_index", u32_type.clone()).into(),
					besl::Node::member("base_meshlet_index", u32_type.clone()).into(),
					besl::Node::member("meshlet_count", u32_type.clone()).into(),
					besl::Node::member("skinned_base_vertex_index", u32_type.clone()).into(),
					besl::Node::member("padding0", u32_type.clone()).into(),
				],
			)
			.into(),
		);
		let view = root.add_child(
			besl::Node::r#struct(
				"View",
				vec![
					besl::Node::member("view", mat4f.clone()).into(),
					besl::Node::member("projection", mat4f.clone()).into(),
					besl::Node::member("view_projection", mat4f.clone()).into(),
					besl::Node::member("inverse_view", mat4f.clone()).into(),
					besl::Node::member("inverse_projection", mat4f.clone()).into(),
					besl::Node::member("inverse_view_projection", mat4f).into(),
					besl::Node::member("fov", vec2f.clone()).into(),
					besl::Node::member("near", f32_type.clone()).into(),
					besl::Node::member("far", f32_type).into(),
				],
			)
			.into(),
		);
		let meshlet = root.add_child(
			besl::Node::r#struct(
				"Meshlet",
				vec![
					besl::Node::member("primitive_offset", u32_type.clone()).into(),
					besl::Node::member("triangle_offset", u32_type.clone()).into(),
					besl::Node::member("primitive_count", u32_type.clone()).into(),
					besl::Node::member("triangle_count", u32_type.clone()).into(),
					besl::Node::member("center_radius", vec4f.clone()).into(),
					besl::Node::member("cone_apex_cutoff", vec4f.clone()).into(),
					besl::Node::member("cone_axis", vec4f.clone()).into(),
				],
			)
			.into(),
		);
		let light = root.add_child(
			besl::Node::r#struct(
				"Light",
				vec![
					besl::Node::member("position", vec4f.clone()).into(),
					besl::Node::member("color", vec4f.clone()).into(),
					besl::Node::member("direction", vec4f).into(),
					besl::Node::member("cone_cosines", vec2f).into(),
					besl::Node::member("type", u32_type.clone()).into(),
					besl::Node::array("cascades", u32_type.clone(), 8),
					besl::Node::member("_padding", u32_type.clone()).into(),
				],
			)
			.into(),
		);

		let mesh_buffer = vec![besl::Node::array("meshes", mesh, 1024)];
		let view_buffer = vec![besl::Node::array("views", view, 8)];
		let meshlet_buffer = vec![besl::Node::array("meshlets", meshlet, 1024)];
		let lighting_buffer = vec![
			besl::Node::member("light_count", u32_type.clone()).into(),
			besl::Node::array("_light_count_padding", u32_type, 3),
			besl::Node::array("lights", light, 16),
		];

		for (target, mesh_stride) in [
			(StorageLayoutTarget::Hlsl, 80),
			(StorageLayoutTarget::Msl, 96),
			(StorageLayoutTarget::GlslScalar, 80),
		] {
			assert_eq!(
				reflected_storage_buffer_stride_for_target(&mesh_buffer, target),
				Ok(mesh_stride)
			);
			assert_eq!(reflected_storage_buffer_stride_for_target(&view_buffer, target), Ok(400));
			assert_eq!(reflected_storage_buffer_stride_for_target(&meshlet_buffer, target), Ok(64));
			assert_eq!(reflected_storage_buffer_stride_for_target(&lighting_buffer, target), Ok(1552));
		}
	}

	#[test]
	fn sampled_texture_shapes_and_descriptor_counts_are_preserved() {
		let root = besl::Node::root();
		let void = root.get_child("void").expect("Expected the built-in void type");
		let main: besl::NodeReference = besl::Node::function(
			"main",
			Vec::new(),
			void,
			vec![besl::Node::binding_array(
				"volumes",
				besl::BindingTypes::CombinedImageSampler {
					format: "Texture3D".to_string(),
				},
				0,
				true,
				false,
				3,
			)
			.into()],
		)
		.into();

		let bindings = ProgramEvaluation::from_main(&main)
			.expect("Expected sampled binding metadata to evaluate")
			.into_bindings();
		assert_eq!(bindings[0].count, 3);
		assert_eq!(
			bindings[0].kind,
			BindingKind::CombinedImageSampler {
				view: TextureView::Texture3D
			}
		);
	}

	#[test]
	fn bindings_from_program() {
		let script = r#"
		main: fn () -> void {
			buff;
			image;
			texture;
		}
		"#;

		let mut root_node = besl::Node::root();

		let float_type = root_node.get_child("f32").unwrap();

		root_node.add_children(vec![
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("member", float_type).into()],
				},
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"image",
				besl::BindingTypes::Image {
					format: "r8".to_string(),
				},
				1,
				false,
				true,
			)
			.into(),
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: "".to_string() },
				2,
				true,
				false,
			)
			.into(),
		]);

		let program_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");
		let bindings = evaluation.bindings();

		assert_eq!(bindings.len(), 3);
	}

	#[test]
	fn opacity_is_opaque_when_non_local_output_is_referenced() {
		let script = r#"
		main: fn () -> void {
			output;
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec3f_type = root_node.get_child("vec3f").unwrap();
		root_node.add_child(besl::Node::output("output", vec3f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Opaque);
	}

	#[test]
	fn opacity_is_unknown_when_output_is_shadowed_locally() {
		let script = r#"
		main: fn () -> void {
			let output: vec3f = vec3f(1.0, 0.0, 0.0);
			output;
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec3f_type = root_node.get_child("vec3f").unwrap();
		root_node.add_child(besl::Node::output("output", vec3f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Unknown);
	}

	#[test]
	fn opacity_is_unknown_when_main_contains_raw_code() {
		let mut root_node = besl::Node::root();
		let return_type = root_node.get_child("void").unwrap();
		let main = besl::Node::function(
			"main",
			Vec::new(),
			return_type,
			vec![besl::Node::glsl("output = vec3f(1.0, 0.0, 0.0);".to_string(), Vec::new(), Vec::new()).into()],
		);
		root_node.add_child(main.into());

		let program_node: besl::NodeReference = root_node.into();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Unknown);
	}

	#[test]
	fn opacity_is_non_opaque_when_output_vec4f_w_is_not_one() {
		let script = r#"
		main: fn () -> void {
			output = vec4f(1.0, 0.0, 0.0, 0.5);
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec4f_type = root_node.get_child("vec4f").unwrap();
		root_node.add_child(besl::Node::output("output", vec4f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::NonOpaque);
	}

	#[test]
	fn opacity_is_opaque_when_output_vec4f_w_is_one() {
		let script = r#"
		main: fn () -> void {
			output = vec4f(1.0, 0.0, 0.0, 1.0);
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec4f_type = root_node.get_child("vec4f").unwrap();
		root_node.add_child(besl::Node::output("output", vec4f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Opaque);
	}

	#[test]
	fn opacity_vec4f_with_vec3f_first_param_uses_w_for_opacity() {
		fn evaluate(w: &str) -> OpacityEvaluation {
			let mut root_node = besl::Node::root();
			let void_type = root_node.get_child("void").unwrap();
			let vec3f_type = root_node.get_child("vec3f").unwrap();
			let vec4f_type = root_node.get_child("vec4f").unwrap();

			let output_node: besl::NodeReference = besl::Node::output("output", vec4f_type.clone(), 0).into();

			let vec3f_call = besl::Node::expression(besl::Expressions::FunctionCall {
				function: vec3f_type,
				parameters: vec![
					besl::Node::expression(besl::Expressions::Literal {
						value: "1.0".to_string(),
					})
					.into(),
					besl::Node::expression(besl::Expressions::Literal {
						value: "0.0".to_string(),
					})
					.into(),
					besl::Node::expression(besl::Expressions::Literal {
						value: "0.0".to_string(),
					})
					.into(),
				],
			})
			.into();

			let vec4f_call = besl::Node::expression(besl::Expressions::FunctionCall {
				function: vec4f_type,
				parameters: vec![
					vec3f_call,
					besl::Node::expression(besl::Expressions::Literal { value: w.to_string() }).into(),
				],
			})
			.into();

			let output_member = besl::Node::expression(besl::Expressions::Member {
				name: "output".to_string(),
				source: output_node.clone(),
			})
			.into();

			let assignment = besl::Node::expression(besl::Expressions::Operator {
				operator: besl::Operators::Assignment,
				left: output_member,
				right: vec4f_call,
			})
			.into();

			let main = besl::Node::function("main", Vec::new(), void_type, vec![assignment]).into();

			root_node.add_children(vec![output_node, main]);

			let program_node: besl::NodeReference = root_node.into();
			let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");
			evaluation.opacity()
		}

		assert_eq!(evaluate("1.0"), OpacityEvaluation::Opaque);
		assert_eq!(evaluate("0.5"), OpacityEvaluation::NonOpaque);
	}
}
