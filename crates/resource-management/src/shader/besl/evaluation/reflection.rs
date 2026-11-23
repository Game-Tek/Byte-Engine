
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
	/// The layer that declares the binding, by its path, or `None` for the program's own layout. See [`SetUsage`].
	pub layout: Option<String>,
}

/// The `SetUsage` struct describes one descriptor set layout a BESL program declares, so a backend can lay the set
/// out once for every program that shares it, and the renderer can bind it as a whole.
///
/// A layout holds every binding its layer declares, whether or not this program reads it, because the
/// native set must look the same to every program. `used` records which of them this program can reach, and only
/// those count as the program's resource accesses. Get the sets of a program from
/// [`ProgramEvaluation::sets`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetUsage {
	/// The path of the layer that declares the set, or `None` for the program's own layout.
	pub layout: Option<String>,
	/// Every binding of the layout, ordered by slot.
	pub bindings: Vec<BindingUsage>,
	/// Bit `i` is set when code reachable from `main` uses `bindings[i]`.
	pub used: u64,
}

impl SetUsage {
	/// The most bindings one layout can declare, the width of [`Self::used`].
	pub const MAX_BINDINGS: usize = u64::BITS as usize;
}

/// The `ProgramSet` struct holds the binding nodes of one descriptor set layout a program declares, in the order
/// [`SetUsage`] reflects them, so a backend emits exactly the layout that reflection reports.
pub(crate) struct ProgramSet {
	/// The path of the layer that declares the set, or `None` for the program's own layout.
	pub(crate) layout: Option<String>,
	/// Every binding of the layout, ordered by slot.
	pub(crate) nodes: Vec<besl::NodeId>,
	/// Bit `i` is set when code reachable from `main` uses `nodes[i]`.
	pub(crate) used: u64,
}

/// The `BindingKind` enum identifies the descriptor category declared by a BESL binding.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum BindingKind {
	/// A structured storage buffer. Read-only access does not change the descriptor category.
	StorageBuffer,
	/// A texture the shader samples. `sampler` holds the state a BESL `sampler` declaration fixes, in which case the
	/// application binds only the image; `None` means the application binds a sampler with it.
	CombinedImageSampler {
		view: TextureView,
		sampler: Option<besl::SamplerState>,
	},
	/// An image the shader writes, and may also read, without a sampler.
	StorageImage,
	/// An image the shader only reads, without a sampler. It binds without storage usage, so Metal can keep it
	/// losslessly compressed.
	SampledImage,
	/// A top-level acceleration structure that ray queries test rays against. Shaders only read it.
	AccelerationStructure,
}

/// The `RayPayload` struct identifies the payload a ray tracing pipeline passes from its ray generation shader's
/// `invoke` calls to its closest-hit shaders. Pipeline assembly compares the payloads of a pipeline's shaders and
/// rejects a mismatch, because Metal links shaders whose payloads differ without an error.
#[derive(
	Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct RayPayload {
	/// The name of the payload struct.
	pub name: String,
	/// The bytes one payload occupies in the active backend's storage layout.
	pub size: u32,
	/// An FNV-1a hash of the payload's name and its members' names and types, in declaration order. Two shaders pass the
	/// same payload exactly when their hashes are equal.
	pub layout_hash: u64,
}

/// The FNV-1a offset basis, which starts a hash of no bytes.
pub const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// Folds `bytes` into an FNV-1a `hash`. The result is stable across processes and builds, so baked assets can store it.
pub fn fnv1a(hash: u64, bytes: &[u8]) -> u64 {
	const PRIME: u64 = 0x0000_0100_0000_01b3;
	bytes
		.iter()
		.fold(hash, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(PRIME))
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
	visited: besl::NodeSet,
	/// The node of each collected binding, in collection order.
	nodes: Vec<besl::NodeId>,
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
fn reflected_storage_buffer_stride(program: &besl::Program, members: &[besl::NodeId]) -> Result<u32, String> {
	reflected_storage_buffer_stride_for_target(program, members, StorageLayoutTarget::current())
}

/// Reflects the stride of one element in an array buffer.
fn reflected_array_buffer_stride(program: &besl::Program, element: &besl::NodeId) -> Result<u32, String> {
	reflected_array_buffer_stride_for_target(program, element, StorageLayoutTarget::current())
}

/// Reflects an array-buffer element, fixed-size or runtime-length, using the selected backend's emitted layout.
pub(super) fn reflected_array_buffer_stride_for_target(
	program: &besl::Program,
	element: &besl::NodeId,
	target: StorageLayoutTarget,
) -> Result<u32, String> {
	// User structs are emitted as packed element structs. Built-in records such as `vec3f` and `mat4x3f` have fields
	// too, but are emitted as packed scalar or vector types.
	let user_struct_fields = {
		let element = &program[element];
		match element.node() {
			besl::Nodes::Struct { name, fields, .. }
				if !fields.is_empty() && primitive_storage_layout(name, target).is_none() =>
			{
				Some(fields.clone())
			}
			_ => None,
		}
	};
	let mut visiting = besl::NodeSet::default();
	let layout = if let Some(fields) = user_struct_fields {
		super::byte_windows::validate_byte_windows(program, &fields)?;
		reflected_storage_members_layout(program, &fields, target, &mut visiting)?
	} else if target == StorageLayoutTarget::Hlsl
		&& program[element]
			.get_name()
			.and_then(crate::shader::besl::backends::hlsl::hlsl_narrow_element)
			.is_some()
	{
		// DX12 exposes narrow arrays as atomically addressable 32-bit words,
		// even though native u16 values remain two bytes in every other HLSL layout.
		StorageLayout { size: 4, alignment: 4 }
	} else {
		reflected_storage_type_layout(program, element, target, &mut visiting)?
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
	program: &besl::Program,
	members: &[besl::NodeId],
	target: StorageLayoutTarget,
) -> Result<u32, String> {
	if members.is_empty() {
		return Err(
			"Empty storage-buffer layout. The most likely cause is that the binding type declares no addressable members."
				.to_string(),
		);
	}

	super::byte_windows::validate_byte_windows(program, members)?;
	// The lexer lowers a lone fixed-array member to an array buffer, so these buffers are always wrapper structs.
	let size = reflected_storage_members_layout(program, members, target, &mut besl::NodeSet::default())?.size;

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
	program: &besl::Program,
	members: &[besl::NodeId],
	target: StorageLayoutTarget,
	visiting: &mut besl::NodeSet,
) -> Result<StorageLayout, String> {
	let mut size = 0usize;
	let mut alignment = 1usize;
	for member in members {
		let member = &program[member];
		let besl::Nodes::Member { name, r#type, count } = member.node() else {
			return Err(
				"Unsupported storage-buffer member. The most likely cause is that a buffer layout contains a node other than a named member."
					.to_string(),
			);
		};
		let element = if target == StorageLayoutTarget::Hlsl && count.is_some() && program[r#type].get_name() == Some("u8") {
			// DX12 declares a `u8` array member as a `uint` array, one 32-bit word per element.
			StorageLayout { size: 4, alignment: 4 }
		} else {
			reflected_storage_type_layout(program, r#type, target, visiting)?
		};
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
	program: &besl::Program,
	r#type: &besl::NodeId,
	target: StorageLayoutTarget,
	visiting: &mut besl::NodeSet,
) -> Result<StorageLayout, String> {
	let type_borrow = &program[r#type];
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

	if !visiting.insert(*r#type) {
		return Err(format!(
			"Recursive storage-buffer type '{type_name}'. The most likely cause is that a shader struct contains itself."
		));
	}
	let layout = reflected_storage_members_layout(program, &fields, target, visiting);
	visiting.remove(*r#type);
	layout
}

/// Returns the backend layout for one built-in BESL storage type.
pub(super) fn primitive_storage_layout(type_name: &str, target: StorageLayoutTarget) -> Option<StorageLayout> {
	let (size, alignment) = match (target, type_name) {
		// Metal keeps its native column alignment for matrices other than `mat4x3f`.
		(StorageLayoutTarget::Msl, "mat2f") => (16, 8),
		(StorageLayoutTarget::Msl, "mat3f") => (48, 16),
		(StorageLayoutTarget::Msl, "mat4f") => (64, 16),
		// Every other type has one layout on every backend. Metal stores vectors as packed types, native u16 values keep
		// their two-byte object representation and scalar alignment, and DX12 stores single `u8` members in 2-byte
		// windows (see `byte_windows`). A linked program has no `bool` members: they are packed into `u8` members.
		(_, "u8") => (1, 1),
		(_, "u16" | "f16") => (2, 2),
		(_, "u32" | "atomicu32" | "i32" | "atomici32" | "f32") => (4, 4),
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
	sets: Vec<SetUsage>,
	opacity: OpacityEvaluation,
	ray_payload: Option<RayPayload>,
}

impl ProgramEvaluation {
	/// Reflects every declared binding while evaluating code behavior from reachable `main`.
	pub fn from_program(program: &mut besl::Program) -> Result<Self, String> {
		let main = program.get_main().ok_or_else(|| {
			"Main function not found. The program description likely does not define a `main` function.".to_string()
		})?;

		besl::optimization::optimize(program, &main);

		let root = program.root();
		Ok(Self {
			bindings: collect_bindings(program, &root)?,
			sets: collect_sets(program, &root)?,
			opacity: evaluate_opacity(program, &main),
			ray_payload: reflect_ray_payload(program, &main)?,
		})
	}

	pub fn from_main(program: &mut besl::Program, main_function_node: &besl::NodeId) -> Result<Self, String> {
		{
			let node_borrow = &program[main_function_node];
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

		besl::optimization::optimize(program, main_function_node);

		let bindings = collect_bindings(program, main_function_node)?;
		let sets = collect_sets(program, main_function_node)?;

		let opacity = evaluate_opacity(program, main_function_node);
		let ray_payload = reflect_ray_payload(program, main_function_node)?;

		Ok(Self {
			bindings,
			sets,
			opacity,
			ray_payload,
		})
	}

	pub fn bindings(&self) -> &[BindingUsage] {
		&self.bindings
	}

	/// Returns the descriptor set layouts the program declares. Next, persist them with
	/// [`crate::resources::material::SetInterface`].
	pub fn into_sets(self) -> Vec<SetUsage> {
		self.sets
	}

	pub fn into_bindings(self) -> Vec<BindingUsage> {
		self.bindings
	}

	pub fn opacity(&self) -> OpacityEvaluation {
		self.opacity
	}

	/// Returns the payload the program passes through ray tracing: the one a closest-hit `main` takes, or the one its
	/// `invoke` calls pass. Next, persist it in [`crate::resources::material::ShaderInterface`].
	pub fn ray_payload(&self) -> Option<&RayPayload> {
		self.ray_payload.as_ref()
	}
}

/// Reflects the payload a program passes through ray tracing, or `None` for a program that neither is a closest hit
/// nor calls `invoke`.
fn reflect_ray_payload(program: &besl::Program, main: &besl::NodeId) -> Result<Option<RayPayload>, String> {
	// Entry normalization leaves a closest hit's payload as the only parameter of `main`.
	let closest_hit_payload = match program[main].node() {
		besl::Nodes::Function { params, .. } => params.iter().find_map(|parameter| match program[parameter].node() {
			besl::Nodes::Parameter { r#type, .. } => Some(*r#type),
			_ => None,
		}),
		_ => None,
	};
	// The lexer makes every `invoke` call of a program pass one payload type, so the first call names it.
	let payload_type = closest_hit_payload.or_else(|| {
		crate::shader::besl::graph::dependency_order(program, main)
			.iter()
			.find_map(|node| match program[node].node() {
				besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) => {
					match program[intrinsic].node() {
						besl::Nodes::Intrinsic { name, r#return, .. } if name == "invoke" => Some(*r#return),
						_ => None,
					}
				}
				_ => None,
			})
	});
	payload_type
		.map(|payload_type| describe_ray_payload(program, &payload_type))
		.transpose()
}

/// Describes one payload struct, which the lexer checked holds only scalar and vector members.
fn describe_ray_payload(program: &besl::Program, payload_type: &besl::NodeId) -> Result<RayPayload, String> {
	let payload = &program[payload_type];
	let besl::Nodes::Struct { name, fields, .. } = payload.node() else {
		return Err(
			"Invalid ray tracing payload. The most likely cause is a payload type that isn't a struct, which the BESL lexer should have rejected. Declare the payload as a struct of scalars and vectors."
				.to_string(),
		);
	};
	let mut layout_hash = fnv1a(FNV1A_OFFSET_BASIS, name.as_bytes());
	for field in fields {
		let field = &program[field];
		let besl::Nodes::Member { name, r#type, .. } = field.node() else {
			continue;
		};
		// Separators keep one name from running into the next.
		layout_hash = fnv1a(layout_hash, &[0]);
		layout_hash = fnv1a(layout_hash, name.as_bytes());
		layout_hash = fnv1a(layout_hash, &[0]);
		layout_hash = fnv1a(layout_hash, program[r#type].get_name().unwrap_or("unknown").as_bytes());
	}
	Ok(RayPayload {
		name: name.clone(),
		size: reflected_array_buffer_stride(program, payload_type)
			.map_err(|error| format!("Failed to reflect ray tracing payload '{name}'. {error}"))?,
		layout_hash,
	})
}

/// Collects sorted binding metadata while sharing repeated references and rejecting distinct slot aliases.
pub(crate) fn collect_bindings(program: &besl::Program, node: &besl::NodeId) -> Result<Vec<BindingUsage>, String> {
	let mut bindings = collect_binding_nodes(program, node)?
		.iter()
		.map(|node| reflect_binding(program, node))
		.collect::<Result<Vec<_>, _>>()?;
	bindings.sort_by(|a, b| (a.layout.as_deref(), a.slot).cmp(&(b.layout.as_deref(), b.slot)));
	Ok(bindings)
}

/// Groups every binding declared under `scope` into the descriptor set layouts it declares, and reflects them.
///
/// See [`collect_set_nodes`] for the grouping rules.
pub(crate) fn collect_sets(program: &besl::Program, scope: &besl::NodeId) -> Result<Vec<SetUsage>, String> {
	collect_set_nodes(program, scope)?
		.into_iter()
		.map(|set| {
			Ok(SetUsage {
				layout: set.layout,
				bindings: set
					.nodes
					.iter()
					.map(|node| reflect_binding(program, node))
					.collect::<Result<_, String>>()?,
				used: set.used,
			})
		})
		.collect()
}

/// Groups the binding nodes declared under `scope` into the descriptor sets they were placed in when the program
/// linked (see [`besl::place_descriptors`]), in set order.
///
/// `scope` is the root of a linked program, or its `main` function. Each set holds every binding placed in it, ordered by slot,
/// whether or not `main` reaches it, so the native set looks the same to every shader that declares it; a binding
/// counts as used when code reachable from `main` uses it. Given `main` alone, only the bindings it reaches are
/// declared. Without a `main`, every binding counts as used.
///
/// Reflection and every backend take their sets from here, so a lowered shader and its interface cannot disagree.
pub(crate) fn collect_set_nodes(program: &besl::Program, scope: &besl::NodeId) -> Result<Vec<ProgramSet>, String> {
	let declared = collect_binding_nodes(program, scope)?;
	// Usage is only known from the root, which holds both `main` and the bindings it may leave unused.
	let main = if *scope == program.root() { program.get_main() } else { None };
	let reachable = match main {
		Some(main) => Some(collect_binding_nodes(program, &main)?),
		None => None,
	};

	let mut sets = Vec::<ProgramSet>::new();
	for node in declared {
		let (layout, placement) = match program[node].node() {
			besl::Nodes::Binding { layout, placement, .. } => (layout.clone(), *placement),
			_ => continue,
		};
		// A binding of a layer `main` doesn't take belongs to no set of the program.
		let Some(placement) = placement else {
			continue;
		};
		let set = placement.set as usize;
		if sets.len() <= set {
			sets.resize_with(set + 1, || ProgramSet {
				layout: None,
				nodes: Vec::new(),
				used: 0,
			});
		}
		sets[set].layout = layout;
		sets[set].nodes.push(node);
	}

	for set in &mut sets {
		set.nodes.sort_by_key(|node| binding_slot(program, node));

		if set.nodes.len() > SetUsage::MAX_BINDINGS {
			return Err(format!(
				"Descriptor set layout `{}` declares {} bindings, more than the {} one layout can hold. The most likely cause is that one layer groups resources that belong to several descriptor sets.",
				set.layout.as_deref().unwrap_or("<shader>"),
				set.nodes.len(),
				SetUsage::MAX_BINDINGS
			));
		}

		for (index, node) in set.nodes.iter().enumerate() {
			let used = reachable.as_ref().is_none_or(|reachable| reachable.contains(node));
			set.used |= u64::from(used) << index;
		}
	}

	Ok(sets)
}

/// Returns the slot of a binding node, its number within its set.
fn binding_slot(program: &besl::Program, node: &besl::NodeId) -> u32 {
	match program[node].node() {
		besl::Nodes::Binding { slot, .. } => *slot,
		_ => u32::MAX,
	}
}

/// Collects the binding nodes the traversal reaches from `node`, in the order it first reaches each one, after
/// rejecting distinct declarations of one binding.
fn collect_binding_nodes(program: &besl::Program, node: &besl::NodeId) -> Result<Vec<besl::NodeId>, String> {
	let mut state = BindingCollectionState {
		visited: besl::NodeSet::for_program(program),
		nodes: Vec::with_capacity(16),
		error: None,
	};
	build_bindings(program, node, &mut state);
	if let Some(error) = state.error {
		return Err(error);
	}

	Ok(state.nodes)
}

/// Reflects the persisted metadata of one binding node.
fn reflect_binding(program: &besl::Program, node: &besl::NodeId) -> Result<BindingUsage, String> {
	let node = &program[node];
	let besl::Nodes::Binding {
		name,
		slot,
		read,
		write,
		r#type,
		count,
		layout,
		..
	} = node.node()
	else {
		return Err(
			"Invalid binding node. The most likely cause is that reflection collected a node that is not a binding."
				.to_string(),
		);
	};

	let (kind, buffer_stride) = match r#type {
		besl::BindingTypes::Buffer { members } => (
			BindingKind::StorageBuffer,
			Some(
				reflected_storage_buffer_stride(program, members)
					.map_err(|error| format!("Failed to reflect storage-buffer binding '{name}'. {error}"))?,
			),
		),
		besl::BindingTypes::BufferArray { element, .. } => (
			BindingKind::StorageBuffer,
			Some(
				reflected_array_buffer_stride(program, element)
					.map_err(|error| format!("Failed to reflect storage-buffer binding '{name}'. {error}"))?,
			),
		),
		besl::BindingTypes::CombinedImageSampler { format, sampler } => (
			BindingKind::CombinedImageSampler {
				view: match format.as_str() {
					"Texture3D" => TextureView::Texture3D,
					"TextureCube" => TextureView::TextureCube,
					"TextureCubeArray" => TextureView::TextureCubeArray,
					"ArrayTexture2D" => TextureView::Texture2DArray,
					_ => TextureView::Texture2D,
				},
				sampler: *sampler,
			},
			None,
		),
		besl::BindingTypes::Image { .. } if r#type.is_sampled_image(*read, *write) => (BindingKind::SampledImage, None),
		besl::BindingTypes::Image { .. } => (BindingKind::StorageImage, None),
		besl::BindingTypes::AccelerationStructure => (BindingKind::AccelerationStructure, None),
	};

	Ok(BindingUsage {
		name: name.to_string(),
		kind,
		count: count.map_or(1, |count| count.get()),
		slot: *slot,
		buffer_stride,
		read: *read,
		write: *write,
		layout: layout.clone(),
	})
}

// This is the exhaustive BESL-node traversal contract for reflection; splitting it would duplicate child-edge rules.
#[allow(clippy::too_many_lines)]
fn build_bindings(program: &besl::Program, node: &besl::NodeId, state: &mut BindingCollectionState) {
	if state.error.is_some() || !state.visited.insert(*node) {
		return;
	}
	let node_borrow = &program[node];
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => {
			for statement in statements {
				build_bindings(program, statement, state);
			}
		}
		branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => {
			for child in branch.branch_children() {
				build_bindings(program, child, state);
			}
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			build_bindings(program, initializer, state);
			build_bindings(program, condition, state);
			build_bindings(program, update, state);
			for statement in statements {
				build_bindings(program, statement, state);
			}
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			} => {
				build_bindings(program, callable, state);
				for argument in arguments {
					build_bindings(program, argument, state);
				}
			}
			besl::Expressions::IntrinsicCall { arguments, elements, .. } => {
				// Intrinsic lowering emits the instantiated elements, not the definition template.
				for element in arguments.iter().chain(elements) {
					build_bindings(program, element, state);
				}
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				build_bindings(program, left, state);
				build_bindings(program, right, state);
			}
			besl::Expressions::Unary { operand, .. } => build_bindings(program, operand, state),
			besl::Expressions::Ternary {
				condition,
				if_true,
				if_false,
			} => {
				for part in [condition, if_true, if_false] {
					build_bindings(program, part, state);
				}
			}
			besl::Expressions::Expression { elements } => {
				for element in elements {
					build_bindings(program, element, state);
				}
			}
			besl::Expressions::Macro { body, .. } => {
				build_bindings(program, body, state);
			}
			besl::Expressions::Member { source, .. } => {
				build_bindings(program, source, state);
			}
			besl::Expressions::VariableDeclaration { r#type, .. } => {
				build_bindings(program, r#type, state);
			}
			besl::Expressions::Return { value } => {
				// A returned expression can be the only path from main to a resource used by a helper function.
				if let Some(value) = value {
					build_bindings(program, value, state);
				}
			}
			besl::Expressions::Literal { .. }
			| besl::Expressions::Continue
			| besl::Expressions::Break
			| besl::Expressions::Discard => {}
		},
		besl::Nodes::Binding { slot, layout, .. } => {
			let duplicate = state.nodes.iter().any(|collected| {
				matches!(program[collected].node(), besl::Nodes::Binding { slot: collected, layout: collected_layout, .. } if collected == slot && collected_layout == layout)
			});
			if duplicate {
				state.error = Some(format!(
					"Duplicate resource declaration at binding {slot} of set `{}`. The most likely cause is that distinct binding nodes reuse one binding number instead of sharing the same binding reference. Give each binding of a set its own number.",
					layout.as_deref().unwrap_or("<shader>")
				));
			} else {
				state.nodes.push(*node);
			}
		}
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			for element in elements {
				build_bindings(program, element, state);
			}
			build_bindings(program, r#return, state);
		}
		besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			build_bindings(program, nested, state);
		}
		besl::Nodes::Input { format, .. }
		| besl::Nodes::Output { format, .. }
		| besl::Nodes::TaskPayload { format, .. }
		| besl::Nodes::Workgroup { format, .. } => {
			build_bindings(program, format, state);
		}
		besl::Nodes::PushConstant { members: nested } => {
			if let Err(error) = super::push_constant_layout::validate_push_constant_layout(program, nested) {
				state.error = Some(error);
				return;
			}
			for child in nested {
				build_bindings(program, child, state);
			}
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::Scope { children: nested, .. }
		| besl::Nodes::Layer { members: nested, .. } => {
			for child in nested {
				build_bindings(program, child, state);
			}
		}
		// A `use` and a signature only link names; what they name is collected where it is declared.
		besl::Nodes::Use { .. } | besl::Nodes::Signature { .. } => {}
		besl::Nodes::Const { r#type, value, .. } => {
			build_bindings(program, r#type, state);
			build_bindings(program, value, state);
		}
	}
}
