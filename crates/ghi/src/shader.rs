use std::sync::Arc;

use utils::Extent;

use crate::{AccessPolicies, TextureViewTypes};

/// A native shader-source representation accepted by a backend.
pub enum Sources<'a> {
	/// SPIR-V binary.
	SPIRV(&'a [u8]),
	/// DirectX Intermediate Language bytecode for DX12 backends.
	DXIL(&'a [u8]),
	/// HLSL source and entry-point name for DX12 backends.
	HLSL { source: &'a str, entry_point: &'a str },
	/// Compiled Metal library bytes and entry-point name.
	MTLB {
		binary: &'a [u8],
		entry_point: &'a str,
		threadgroup_size: Option<Extent>,
	},
	/// Metal Shading Language source and entry-point name.
	MTL { source: &'a str, entry_point: &'a str },
}

/// A platform-specific shader source for cross-platform compilation.
///
/// Use [`compile`] to select the source for the active backend.
#[derive(Clone, Copy)]
pub enum ShaderSource<'a> {
	/// HLSL source code compiled for DX12.
	Hlsl { source: &'a str, entry_point: &'a str },
	/// Paired GLSL, MSL, and HLSL sources; [`compile`] selects the native variant for the active backend.
	PlatformNative {
		glsl: &'a str,
		msl: &'a str,
		msl_entry_point: &'a str,
		hlsl: &'a str,
		hlsl_entry_point: &'a str,
	},
}

/// A shader source selected and compiled for one backend.
pub enum CompiledShaderSource {
	/// SPIR-V binary compiled from GLSL.
	SPIRV(Vec<u8>),
	/// HLSL source and entry-point name.
	HLSL { source: String, entry_point: String },
	/// Metal shading language source and entry-point name.
	MTL { source: String, entry_point: String },
}

impl CompiledShaderSource {
	pub fn as_source(&self) -> Sources<'_> {
		match self {
			Self::SPIRV(binary) => Sources::SPIRV(binary.as_slice()),
			Self::HLSL { source, entry_point } => Sources::HLSL {
				source: source.as_str(),
				entry_point: entry_point.as_str(),
			},
			Self::MTL { source, entry_point } => Sources::MTL {
				source: source.as_str(),
				entry_point: entry_point.as_str(),
			},
		}
	}
}

/// Compiles a platform-specific shader source into the representation expected by a device.
pub fn compile(name: &str, source: ShaderSource) -> Result<CompiledShaderSource, String> {
	match source {
		ShaderSource::Hlsl { source, entry_point } => Ok(CompiledShaderSource::HLSL {
			source: source.to_string(),
			entry_point: entry_point.to_string(),
		}),
		ShaderSource::PlatformNative {
			glsl,
			msl,
			msl_entry_point,
			hlsl,
			hlsl_entry_point,
		} => {
			if crate::implementation::USES_DX12 {
				Ok(CompiledShaderSource::HLSL {
					source: hlsl.to_string(),
					entry_point: hlsl_entry_point.to_string(),
				})
			} else if crate::implementation::USES_METAL {
				Ok(CompiledShaderSource::MTL {
					source: msl.to_string(),
					entry_point: msl_entry_point.to_string(),
				})
			} else if crate::implementation::USES_VULKAN {
				compile_glsl(name, glsl)
			} else {
				Err("Platform-native shader source does not include a supported backend for this OS. The most likely cause is compiling on an unsupported operating system.".to_string())
			}
		}
	}
}

#[cfg(target_os = "linux")]
fn compile_glsl(name: &str, source: &str) -> Result<CompiledShaderSource, String> {
	let compiler = shaderc::Compiler::new().map_err(|error| {
		format!(
			"GLSL compiler initialization failed. The most likely cause is unavailable shaderc runtime support. Error: {error}"
		)
	})?;
	let mut options = shaderc::CompileOptions::new().map_err(|error| {
		format!(
			"GLSL compiler options could not be created. The most likely cause is unavailable shaderc runtime support. Error: {error}"
		)
	})?;
	options.set_optimization_level(shaderc::OptimizationLevel::Performance);
	options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_4 as u32);
	options.set_target_spirv(shaderc::SpirvVersion::V1_6);
	if cfg!(debug_assertions) {
		options.set_generate_debug_info();
	}

	compiler
		.compile_into_spirv(source, shaderc::ShaderKind::InferFromSource, name, "main", Some(&options))
		.map(|artifact| CompiledShaderSource::SPIRV(artifact.as_binary_u8().to_vec()))
		.map_err(|error| {
			format!(
				"GLSL shader compilation failed for '{name}'. The most likely cause is invalid or unsupported shader source. Error: {error}"
			)
		})
}

#[cfg(not(target_os = "linux"))]
fn compile_glsl(_name: &str, _source: &str) -> Result<CompiledShaderSource, String> {
	Err(
		"GLSL shader compilation requires a Vulkan backend (Linux only). Use a BESL shader for cross-platform support."
			.to_string(),
	)
}

/// The `ResourceSlot` struct identifies one resource in a shader's flat resource namespace.
///
/// Slots are unique across the descriptor set layouts a pipeline declares, so a slot names one resource whichever
/// set holds it. A [`DescriptorSetLayout`] groups the slots that one descriptor set holds, and a write names its
/// resource by slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ResourceSlot(u32);

impl ResourceSlot {
	pub const fn new(slot: u32) -> Self {
		Self(slot)
	}

	pub const fn index(self) -> u32 {
		self.0
	}
}

impl From<u32> for ResourceSlot {
	fn from(slot: u32) -> Self {
		Self::new(slot)
	}
}

/// The native resource category expected at a flat shader slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
	UniformBuffer,
	StorageBuffer,
	SampledImage,
	CombinedImageSampler,
	StorageImage,
	InputAttachment,
	Sampler,
	AccelerationStructure,
}

/// The `ShaderResourceDescriptor` struct defines the complete retained-resource contract used to build a pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShaderResourceDescriptor {
	pub(crate) slot: ResourceSlot,
	pub(crate) kind: ResourceKind,
	pub(crate) count: u32,
	pub(crate) access: AccessPolicies,
	pub(crate) texture_view_type: TextureViewTypes,
	pub(crate) buffer_stride: u32,
}

impl ShaderResourceDescriptor {
	pub const fn new(slot: ResourceSlot, kind: ResourceKind, count: u32, access: AccessPolicies) -> Self {
		assert!(
			count > 0,
			"Invalid shader resource count. The most likely cause is that a shader declared an empty resource array."
		);
		assert!(
			slot.index().checked_add(count).is_some(),
			"Invalid shader resource slot range. The most likely cause is that a resource array extends beyond the flat slot namespace."
		);
		Self {
			slot,
			kind,
			count,
			access,
			texture_view_type: TextureViewTypes::Texture2D,
			buffer_stride: 4,
		}
	}

	pub const fn single(slot: ResourceSlot, kind: ResourceKind, access: AccessPolicies) -> Self {
		Self::new(slot, kind, 1, access)
	}

	pub const fn texture_view_type(mut self, texture_view_type: TextureViewTypes) -> Self {
		self.texture_view_type = texture_view_type;
		self
	}

	pub const fn buffer_stride(mut self, buffer_stride: u32) -> Self {
		self.buffer_stride = buffer_stride;
		self
	}

	pub const fn slot(self) -> ResourceSlot {
		self.slot
	}

	pub const fn kind(self) -> ResourceKind {
		self.kind
	}

	pub const fn count(self) -> u32 {
		self.count
	}

	pub const fn access(self) -> AccessPolicies {
		self.access
	}

	pub const fn texture_view(self) -> TextureViewTypes {
		self.texture_view_type
	}

	pub const fn buffer_element_stride(self) -> u32 {
		self.buffer_stride
	}
}

/// The most resources one [`DescriptorSetLayout`] can declare, the width of [`ShaderDescriptorSet::used`].
pub const MAX_LAYOUT_RESOURCES: usize = u64::BITS as usize;

/// The `DescriptorSetLayout` struct declares the resources one descriptor set holds, so a backend lays the set out
/// once, and every pipeline whose shaders declare the same layout shares the sets created from it.
///
/// Build one layout per group of resources that is bound together, such as a scene's buffers or one pass's inputs.
/// Next, create sets with [`crate::context::ContextCreate::create_descriptor_set`], and declare the layout to each
/// shader that reads it with [`ShaderDescriptorSet`]. Cloning is cheap; clones are the same layout.
#[derive(Clone, Debug)]
pub struct DescriptorSetLayout {
	inner: Arc<DescriptorSetLayoutInner>,
}

#[derive(Debug)]
struct DescriptorSetLayoutInner {
	name: Box<str>,
	identity: u64,
	/// Ordered by slot.
	resources: Box<[ShaderResourceDescriptor]>,
	/// For each resource, how many array elements the resources before it hold.
	element_bases: Box<[u32]>,
	element_count: u32,
}

impl DescriptorSetLayout {
	/// Builds a layout from its resources, in any order. `name` identifies the layout in diagnostics.
	///
	/// # Panics
	///
	/// Panics when two resources reserve the same slot, or when the layout declares more than
	/// [`MAX_LAYOUT_RESOURCES`] resources.
	pub fn new(name: &str, resources: impl IntoIterator<Item = ShaderResourceDescriptor>) -> Self {
		let mut resources = resources.into_iter().collect::<Vec<_>>();
		resources.sort_by_key(|resource| resource.slot());

		assert!(
			resources.len() <= MAX_LAYOUT_RESOURCES,
			"Too many resources in descriptor set layout `{name}`. The most likely cause is that one layout groups resources that belong to several descriptor sets."
		);
		assert!(
			resources
				.windows(2)
				.all(|pair| pair[0].slot().index() + pair[0].count() <= pair[1].slot().index()),
			"Overlapping resource slots in descriptor set layout `{name}`. The most likely cause is that a resource array reserves a slot another resource declares."
		);

		let mut element_bases = Vec::with_capacity(resources.len());
		let mut element_count = 0u32;
		for resource in &resources {
			element_bases.push(element_count);
			element_count = element_count.checked_add(resource.count()).expect(
				"Descriptor set layout element count overflowed. The most likely cause is an invalid resource array count.",
			);
		}

		Self {
			inner: Arc::new(DescriptorSetLayoutInner {
				name: name.into(),
				identity: resources_identity(&resources, 0),
				resources: resources.into(),
				element_bases: element_bases.into(),
				element_count,
			}),
		}
	}

	/// Returns this layout with `signature` made part of its identity.
	///
	/// Two layouts with equal resources are interchangeable to a backend. Pass a signature of what the application
	/// also requires to match, such as the names its shaders give the resources, so that a set built for one layout
	/// is rejected where another one with the same shape was meant.
	pub fn with_signature(self, signature: u64) -> Self {
		Self {
			inner: Arc::new(DescriptorSetLayoutInner {
				name: self.inner.name.clone(),
				identity: resources_identity(&self.inner.resources, signature),
				resources: self.inner.resources.clone(),
				element_bases: self.inner.element_bases.clone(),
				element_count: self.inner.element_count,
			}),
		}
	}

	/// Returns the name diagnostics identify the layout by.
	pub fn name(&self) -> &str {
		&self.inner.name
	}

	/// Returns a value that is equal for two layouts exactly when a set created from one can be bound where a
	/// shader declares the other.
	pub fn identity(&self) -> u64 {
		self.inner.identity
	}

	/// Returns the layout's resources, ordered by slot.
	pub fn resources(&self) -> &[ShaderResourceDescriptor] {
		&self.inner.resources
	}

	/// Returns the position in [`Self::resources`] of the resource declared at `slot`.
	///
	/// A resource array is declared at its first slot.
	pub fn resource_index(&self, slot: ResourceSlot) -> Option<usize> {
		self.inner
			.resources
			.binary_search_by_key(&slot, |resource| resource.slot())
			.ok()
	}

	/// Returns how many array elements the resources before `resource_index` hold, the position of that resource's
	/// first element in a set's dense storage.
	pub(crate) fn element_base(&self, resource_index: usize) -> u32 {
		self.inner.element_bases[resource_index]
	}

	/// Returns how many array elements the layout's resources hold in total.
	pub(crate) fn element_count(&self) -> u32 {
		self.inner.element_count
	}
}

/// Hashes what makes two layouts interchangeable, with FNV-1a.
fn resources_identity(resources: &[ShaderResourceDescriptor], signature: u64) -> u64 {
	const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
	const PRIME: u64 = 0x0000_0100_0000_01b3;

	let mut hash = OFFSET_BASIS;
	let mut write = |value: u64| {
		for byte in value.to_le_bytes() {
			hash = (hash ^ u64::from(byte)).wrapping_mul(PRIME);
		}
	};

	write(signature);
	for resource in resources {
		write(u64::from(resource.slot.index()));
		write(resource.kind as u64);
		write(u64::from(resource.count));
		write(u64::from(resource.access.bits()));
		write(resource.texture_view_type as u64);
		write(u64::from(resource.buffer_stride));
	}

	hash
}

/// The `ShaderDescriptorSet` struct declares one descriptor set layout to a shader, with the resources of it the
/// shader uses.
///
/// A shader declares the whole layout, so every shader that shares it reads one native set. Only the resources a
/// shader uses count as its accesses and need a resource written before it runs. Pass one per layout to
/// [`crate::context::ContextCreate::create_shader`], in the order the shader's native source declares its sets;
/// layouts the shader uses no resource of take no native binding.
#[derive(Clone, Debug)]
pub struct ShaderDescriptorSet {
	layout: DescriptorSetLayout,
	used: u64,
}

impl ShaderDescriptorSet {
	/// Declares `layout` to a shader that uses every resource of it.
	pub fn new(layout: DescriptorSetLayout) -> Self {
		let used = match layout.resources().len() {
			0 => 0,
			count => u64::MAX >> (MAX_LAYOUT_RESOURCES - count),
		};
		Self { layout, used }
	}

	/// Declares `layout` to a shader that uses the resources whose bit is set in `used`: bit `i` is
	/// `layout.resources()[i]`.
	pub fn with_used(layout: DescriptorSetLayout, used: u64) -> Self {
		let declared = Self::new(layout);
		Self {
			used: used & declared.used,
			..declared
		}
	}

	pub fn layout(&self) -> &DescriptorSetLayout {
		&self.layout
	}

	/// Returns the used resources as a mask over [`DescriptorSetLayout::resources`].
	pub fn used(&self) -> u64 {
		self.used
	}

	/// Returns the resources of the layout the shader uses, with their position in the layout.
	pub fn used_resources(&self) -> impl Iterator<Item = (usize, ShaderResourceDescriptor)> + '_ {
		self.layout
			.resources()
			.iter()
			.copied()
			.enumerate()
			.filter(|(index, _)| self.used & (1 << index) != 0)
	}
}

/// Returns the resources a shader uses across its descriptor sets as one list ordered by slot.
///
/// The Vulkan and DX12 backends derive their native layouts from this flat interface, and treat descriptor sets as
/// groups of flat slots. Slots are unique across the sets a pipeline declares, so the list names each resource once.
#[cfg(not(target_os = "macos"))]
pub(crate) fn flatten_used_resources(
	descriptor_sets: impl IntoIterator<Item = ShaderDescriptorSet>,
) -> Vec<ShaderResourceDescriptor> {
	let mut resources = descriptor_sets
		.into_iter()
		.flat_map(|set| set.used_resources().map(|(_, resource)| resource).collect::<Vec<_>>())
		.collect::<Vec<_>>();
	resources.sort_by_key(|resource| resource.slot());
	resources
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_layout_orders_its_resources_by_slot_and_finds_them_by_slot() {
		let layout = DescriptorSetLayout::new(
			"test",
			[
				ShaderResourceDescriptor::new(
					ResourceSlot::new(9),
					ResourceKind::CombinedImageSampler,
					4,
					AccessPolicies::READ,
				),
				ShaderResourceDescriptor::single(ResourceSlot::new(2), ResourceKind::StorageBuffer, AccessPolicies::READ),
				ShaderResourceDescriptor::single(ResourceSlot::new(13), ResourceKind::StorageImage, AccessPolicies::WRITE),
			],
		);

		let slots = layout
			.resources()
			.iter()
			.map(|resource| resource.slot().index())
			.collect::<Vec<_>>();
		assert_eq!(slots, [2, 9, 13]);
		assert_eq!(layout.resource_index(ResourceSlot::new(9)), Some(1));
		// An array is declared at its first slot only.
		assert_eq!(layout.resource_index(ResourceSlot::new(10)), None);
		assert_eq!((layout.element_base(2), layout.element_count()), (5, 6));
	}

	#[test]
	fn layouts_are_interchangeable_only_with_equal_resources_and_signature() {
		let resources = |access| {
			[ShaderResourceDescriptor::single(
				ResourceSlot::new(0),
				ResourceKind::StorageBuffer,
				access,
			)]
		};
		let layout = DescriptorSetLayout::new("first", resources(AccessPolicies::READ));

		assert_eq!(
			layout.identity(),
			DescriptorSetLayout::new("second", resources(AccessPolicies::READ)).identity()
		);
		assert_ne!(
			layout.identity(),
			DescriptorSetLayout::new("first", resources(AccessPolicies::WRITE)).identity()
		);
		assert_ne!(layout.identity(), layout.clone().with_signature(7).identity());
	}

	#[test]
	fn a_shader_set_uses_only_resources_its_layout_declares() {
		let layout = DescriptorSetLayout::new(
			"test",
			[
				ShaderResourceDescriptor::single(ResourceSlot::new(0), ResourceKind::StorageBuffer, AccessPolicies::READ),
				ShaderResourceDescriptor::single(ResourceSlot::new(1), ResourceKind::StorageImage, AccessPolicies::WRITE),
			],
		);

		assert_eq!(ShaderDescriptorSet::new(layout.clone()).used(), 0b11);
		let used = ShaderDescriptorSet::with_used(layout, 0b110);
		assert_eq!(used.used(), 0b10);
		assert_eq!(used.used_resources().map(|(index, _)| index).collect::<Vec<_>>(), [1]);
	}

	#[test]
	fn platform_native_selects_backend_specific_shader_variant() {
		let compiled = compile(
			"platform-native",
			ShaderSource::PlatformNative {
				glsl: "#version 450\n#pragma shader_stage(compute)\nlayout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;\nvoid main() {}",
				msl: "kernel void main0() {}",
				msl_entry_point: "main0",
				hlsl: "[numthreads(1, 1, 1)] void main() {}",
				hlsl_entry_point: "main",
			},
		)
		.expect("Expected platform-native shader selection to compile.");

		if crate::implementation::USES_DX12 {
			assert!(matches!(
				compiled,
				CompiledShaderSource::HLSL { entry_point, .. } if entry_point == "main"
			));
		} else if crate::implementation::USES_METAL {
			assert!(matches!(
				compiled,
				CompiledShaderSource::MTL { entry_point, .. } if entry_point == "main0"
			));
		} else {
			assert!(matches!(compiled, CompiledShaderSource::SPIRV(binary) if !binary.is_empty()));
		}
	}
}
