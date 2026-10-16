//! Descriptor set layouts the renderer shares with its shaders, and name-based binding of resources to them.
//!
//! A layout layer, a `.besl` file that declares descriptors, is the single declaration of one descriptor set: the
//! shaders that list the layer are baked against it, and the renderer builds its [`SetLayout`] from the same file
//! with [`SetLayout::from_layers`]. Resources are bound by the names the layer gives them, so no slot number is
//! repeated in Rust.

use std::sync::Arc;

use ghi::context::ContextCreate;
use ghi::frame::Frame as _;
use resource_management::resources::material::{Binding, BindingKind, SetInterface, TextureView};
use smallvec::SmallVec;

/// Names one layout layer under `assets/rendering` and embeds its source, as [`SetLayout::from_layers`] takes it.
///
/// The name is the layer's path, which diagnostics show.
macro_rules! layer {
	($path:literal) => {
		(
			$path,
			include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/", $path)),
		)
	};
}
pub(crate) use layer;

/// The `SetLayout` struct describes one descriptor set the renderer shares with its shaders: the GHI layout sets are
/// created from, and the name each resource has in BESL.
///
/// Build it from a layout layer with [`Self::from_layers`], or from a published pipeline's interface with
/// [`Self::new`]. Next, create a set with [`Self::create_set`] and bind resources to it by name through
/// [`Self::binder`]. Cloning is cheap; clones are the same layout.
#[derive(Clone)]
pub struct SetLayout {
	interface: Arc<SetInterface>,
	layout: ghi::DescriptorSetLayout,
}

impl SetLayout {
	/// Builds the layout a baked shader declares.
	pub fn new(interface: &SetInterface) -> Self {
		// The signature makes the names part of the layout's identity, so a set built for one layer is rejected where
		// a shader was baked against another version of it.
		let layout = ghi::DescriptorSetLayout::new(&interface.layout, interface.bindings.iter().map(binding_to_descriptor))
			.with_signature(interface.identity());
		Self {
			interface: Arc::new(interface.clone()),
			layout,
		}
	}

	/// Builds the layout the last of `layers` declares. The layers before it are the ones it builds on, lowest first,
	/// as a shader's sidecar lists them. Give each layer as its name, which diagnostics show, and its source, such as
	/// the layer's asset path and `include_str!` of the file.
	///
	/// # Panics
	///
	/// Panics when the layers do not link, or when the last layer declares no descriptor. Both are errors in the
	/// engine's own assets.
	pub fn from_layers(layers: &[(&str, &str)]) -> Self {
		let (name, _) = layers
			.last()
			.expect("No layout layer. The most likely cause is that a descriptor set layout was requested without its layer.");
		let sets = resource_management::shader::besl::evaluation::reflect_layers(layers.iter().copied())
			.unwrap_or_else(|error| panic!("Layout layer `{name}` could not be reflected. {error}"));
		let set = sets
			.into_iter()
			.find(|set| set.layout.as_deref() == Some(*name))
			.unwrap_or_else(|| {
				panic!(
					"Layout layer `{name}` declares no descriptor. The most likely cause is that a layer of shared types was used as a descriptor set layout."
				)
			});
		Self::new(&SetInterface::new(set, name))
	}

	/// Returns each distinct layout of `sets` once, in first-seen order, with the bindings any of its copies uses
	/// marked as used.
	///
	/// A pipeline's stages each declare the layouts they list; this is the pipeline's view of them.
	pub fn merged<'a>(sets: impl IntoIterator<Item = &'a SetLayout>) -> Arc<[SetLayout]> {
		let mut merged = Vec::<SetLayout>::new();
		for set in sets {
			match merged
				.iter_mut()
				.find(|existing| existing.layout.identity() == set.layout.identity())
			{
				Some(existing) if existing.interface.used | set.interface.used != existing.interface.used => {
					existing.interface = Arc::new(SetInterface {
						used: existing.interface.used | set.interface.used,
						..(*existing.interface).clone()
					});
				}
				Some(_) => {}
				None => merged.push(set.clone()),
			}
		}
		merged.into()
	}

	/// Returns the GHI layout, which descriptor sets are created from and shaders declare.
	pub fn layout(&self) -> &ghi::DescriptorSetLayout {
		&self.layout
	}

	/// Returns the layout's bindings, with the names BESL gives them.
	pub fn interface(&self) -> &SetInterface {
		&self.interface
	}

	/// Returns the slot of the resource BESL names `name`.
	///
	/// # Panics
	///
	/// Panics when the layout declares no such resource.
	pub fn slot(&self, name: &str) -> ghi::ResourceSlot {
		let binding = self.interface.binding(name).unwrap_or_else(|| {
			panic!(
				"Descriptor set layout `{}` declares no resource named `{name}`. The most likely cause is that the name does not match the layout layer's descriptor.",
				self.interface.layout,
			)
		});
		ghi::ResourceSlot::new(binding.slot)
	}

	/// Creates an empty descriptor set for this layout. Next, write its resources through [`Self::binder`].
	pub fn create_set(&self, context: &mut impl ContextCreate, name: &str) -> ghi::DescriptorSetHandle {
		context.create_descriptor_set(Some(name), &self.layout)
	}

	/// Returns a binder that builds the writes of `set`, which must have been created from this layout.
	pub fn binder(&self, set: ghi::DescriptorSetHandle) -> SetBinder<'_> {
		SetBinder { layout: self, set }
	}
}

/// The `SetBinder` struct builds the descriptor writes of one set by BESL resource name.
///
/// Get one from [`SetLayout::binder`], then pass its writes to `ghi::context::Context::write` or
/// `ghi::frame::Frame::write`. Every method panics when the layout declares no resource of that name.
#[derive(Clone, Copy)]
pub struct SetBinder<'a> {
	layout: &'a SetLayout,
	set: ghi::DescriptorSetHandle,
}

impl SetBinder<'_> {
	/// Returns the set the writes target.
	pub fn set(&self) -> ghi::DescriptorSetHandle {
		self.set
	}

	/// Returns the slot of the resource named `name`, for the writes this binder has no method for.
	pub fn slot(&self, name: &str) -> ghi::ResourceSlot {
		self.layout.slot(name)
	}

	/// Binds a buffer.
	pub fn buffer(&self, name: &str, buffer: impl Into<ghi::BaseBufferHandle>) -> ghi::DescriptorWrite {
		ghi::DescriptorWrite::buffer(self.set, self.slot(name), buffer.into())
	}

	/// Binds a storage image a shader loads or stores.
	pub fn image(&self, name: &str, image: impl Into<ghi::BaseImageHandle>) -> ghi::DescriptorWrite {
		ghi::DescriptorWrite::image(self.set, self.slot(name), image, ghi::Layouts::General)
	}

	/// Binds one mip of a storage image.
	/// Binds one element of an arrayed storage image, such as a debug target pool slot.
	pub fn image_element(&self, name: &str, image: impl Into<ghi::BaseImageHandle>, index: u32) -> ghi::DescriptorWrite {
		ghi::DescriptorWrite::image(self.set, self.slot(name), image, ghi::Layouts::General).with_array_element(index)
	}

	pub fn image_mip(&self, name: &str, image: impl Into<ghi::BaseImageHandle>, mip_level: u32) -> ghi::DescriptorWrite {
		ghi::DescriptorWrite::image_mip(self.set, self.slot(name), image, ghi::Layouts::General, mip_level)
	}

	/// Binds a texture a shader samples through `sampler`.
	pub fn sampled(
		&self,
		name: &str,
		image: impl Into<ghi::BaseImageHandle>,
		sampler: ghi::SamplerHandle,
	) -> ghi::DescriptorWrite {
		ghi::DescriptorWrite::combined_image_sampler(self.set, self.slot(name), image, sampler, ghi::Layouts::Read)
	}

	/// Binds the copy of a per-frame texture that belongs to a relative frame, such as `-1` for the previous frame.
	pub fn sampled_with_frame(
		&self,
		name: &str,
		image: ghi::DynamicImageHandle,
		sampler: ghi::SamplerHandle,
		frame_offset: i32,
	) -> ghi::DescriptorWrite {
		ghi::DescriptorWrite::combined_image_sampler_with_frame(
			self.set,
			self.slot(name),
			image,
			sampler,
			ghi::Layouts::Read,
			frame_offset,
		)
	}

	/// Binds one element of a texture array.
	pub fn sampled_element(
		&self,
		name: &str,
		image: impl Into<ghi::BaseImageHandle>,
		sampler: ghi::SamplerHandle,
		index: u32,
	) -> ghi::DescriptorWrite {
		ghi::DescriptorWrite::combined_image_sampler_array(self.set, self.slot(name), image, sampler, ghi::Layouts::Read, index)
	}
}

/// Converts one persisted binding into the GHI resource a layout declares for it.
fn binding_to_descriptor(binding: &Binding) -> ghi::ShaderResourceDescriptor {
	let mut access = ghi::AccessPolicies::empty();
	access.set(ghi::AccessPolicies::READ, binding.read);
	access.set(ghi::AccessPolicies::WRITE, binding.write);
	let descriptor =
		|kind| ghi::ShaderResourceDescriptor::new(ghi::ResourceSlot::new(binding.slot), kind, binding.count, access);
	match binding.kind {
		BindingKind::StorageBuffer => descriptor(ghi::ResourceKind::StorageBuffer).buffer_stride(
			binding
				.buffer_stride
				.expect("Missing persisted storage-buffer stride. The most likely cause is a stale shader interface resource."),
		),
		BindingKind::CombinedImageSampler { view } => {
			descriptor(ghi::ResourceKind::CombinedImageSampler).texture_view_type(match view {
				TextureView::Texture2D => ghi::TextureViewTypes::Texture2D,
				TextureView::Texture2DArray => ghi::TextureViewTypes::Texture2DArray,
				TextureView::TextureCube => ghi::TextureViewTypes::TextureCube,
				TextureView::TextureCubeArray => ghi::TextureViewTypes::TextureCubeArray,
				TextureView::Texture3D => ghi::TextureViewTypes::Texture3D,
			})
		}
		BindingKind::StorageImage => descriptor(ghi::ResourceKind::StorageImage),
	}
}

/// The most descriptor sets [`create_sets`] makes for one pipeline: the shader's own and the layout layers it lists.
pub const MAX_PIPELINE_SETS: usize = 4;

/// The `PipelineSets` struct holds the descriptor sets [`create_sets`] made for one pipeline, to bind when the
/// pipeline is recorded. A fixed capacity keeps it copyable into render commands.
#[derive(Clone, Copy, Default)]
pub struct PipelineSets {
	/// The set of each layout the pipeline uses, filled from the front.
	sets: [Option<ghi::DescriptorSetHandle>; MAX_PIPELINE_SETS],
}

impl PipelineSets {
	/// Returns the sets to pass to `bind_descriptor_sets`.
	pub fn handles(&self) -> SmallVec<[ghi::DescriptorSetHandle; MAX_PIPELINE_SETS]> {
		self.sets.iter().flatten().copied().collect()
	}
}

/// The `PublishedSets` struct holds the descriptor sets of one pipeline whose layouts are only known once it is
/// published, and makes them again when the pipeline is rebuilt.
///
/// Keep one next to the pipeline's [`PipelineRef`](crate::rendering::PipelineRef) and call [`Self::get`] each
/// frame before recording.
#[derive(Clone, Copy, Default)]
pub struct PublishedSets {
	/// The pipeline revision the sets, or the failure, belong to.
	revision: u64,
	sets: Option<PipelineSets>,
	failed: bool,
}

impl PublishedSets {
	/// Returns the pipeline's sets, creating them with [`create_sets`] the first time the pipeline is available and
	/// after each rebuild. Returns `None` while the pipeline is unavailable, or when `resources` do not match its
	/// layouts, which is logged once per revision.
	pub fn get(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		pipeline: crate::rendering::PipelineRef,
		name: &str,
		resources: &[Resource],
	) -> Option<PipelineSets> {
		let revision = pipeline_manager.revision(pipeline);
		if self.revision != revision {
			*self = Self {
				revision,
				..Self::default()
			};
		}
		if self.sets.is_some() || self.failed {
			return self.sets;
		}

		let layouts = pipeline_manager.sets(pipeline)?;
		match create_sets(frame, name, &layouts, resources) {
			Ok(sets) => self.sets = Some(sets),
			Err(error) => {
				log::error!("Descriptor sets `{name}` could not be created: {error}");
				self.failed = true;
			}
		}
		self.sets
	}
}

/// Creates the descriptor sets a published pipeline reads, one per layout its shaders use, and binds `resources` to
/// them by BESL name.
///
/// Use it for sets whose layout is only known once the pipeline is published, such as a shader's own descriptors.
/// `sets` are the pipeline's layouts and `name` labels the created sets. Every binding the pipeline reaches must take
/// exactly one resource of its kind; a binding it does not reach may be left out. Next, bind
/// [`PipelineSets::handles`] after the pipeline.
pub fn create_sets(
	frame: &mut ghi::implementation::Frame,
	name: &str,
	sets: &[SetLayout],
	resources: &[Resource],
) -> Result<PipelineSets, String> {
	validate_binding_schema(sets)?;
	validate_resources(sets, resources)?;

	let mut pipeline_sets = PipelineSets::default();
	let mut writes = SmallVec::<[ghi::DescriptorWrite; 8]>::new();
	// A layout the pipeline reaches no binding of needs no set.
	for (index, set) in sets.iter().filter(|set| set.interface().used != 0).enumerate() {
		let Some(slot) = pipeline_sets.sets.get_mut(index) else {
			return Err(
				"Too many descriptor sets. The most likely cause is that the shader lists more layout layers with descriptors than one pipeline binds by name."
					.to_string(),
			);
		};
		let descriptor_set = set.create_set(frame, name);
		*slot = Some(descriptor_set);
		for binding in &set.interface().bindings {
			// A binding the pipeline does not reach may be left without a resource.
			if let Some(resource) = resources.iter().find(|resource| resource.name() == binding.name) {
				resource.descriptor_writes(descriptor_set, ghi::ResourceSlot::new(binding.slot), &mut writes);
			}
		}
	}
	frame.write(&writes);

	Ok(pipeline_sets)
}

/// The most elements [`Resource::image_array`] binds, which is the most debug targets a sink's pool holds.
pub const MAX_IMAGE_ARRAY_ELEMENTS: usize = 15;

/// The `Resource` enum names one concrete resource for a BESL binding, for [`create_sets`] to bind by name.
#[derive(Clone, Copy)]
pub enum Resource {
	Buffer(&'static str, ghi::BaseBufferHandle),
	PlannedBuffer(&'static str, ghi::BaseBufferHandle),
	Image(&'static str, ghi::ImageOrSwapchain),
	/// The first `count` images of an arrayed storage-image binding, element by element.
	ImageArray(&'static str, [Option<ghi::BaseImageHandle>; MAX_IMAGE_ARRAY_ELEMENTS]),
	CombinedImageSampler(&'static str, ghi::BaseImageHandle, ghi::SamplerHandle, ghi::Layouts),
}

impl Resource {
	pub fn buffer(name: &'static str, buffer: impl Into<ghi::BaseBufferHandle>) -> Self {
		Self::Buffer(name, buffer.into())
	}

	/// Keeps a buffer ready for a BESL binding that is intentionally not reachable yet.
	pub fn planned_buffer(name: &'static str, buffer: impl Into<ghi::BaseBufferHandle>) -> Self {
		Self::PlannedBuffer(name, buffer.into())
	}

	/// Binds a storage-image destination without exposing whether it is an image or swapchain.
	pub fn image(name: &'static str, image: impl Into<ghi::ImageOrSwapchain>) -> Self {
		Self::Image(name, image.into())
	}

	/// Binds `images` to the first elements of an arrayed storage-image binding, such as the debug target pool a
	/// shader declares as `debug_view_targets`. Elements past them stay unbound, so the shader must not index them.
	///
	/// # Panics
	///
	/// Panics with more than [`MAX_IMAGE_ARRAY_ELEMENTS`] images.
	pub fn image_array(name: &'static str, images: &[ghi::BaseImageHandle]) -> Self {
		assert!(
			images.len() <= MAX_IMAGE_ARRAY_ELEMENTS,
			"Image array binding `{name}` has {} images. The most likely cause is a debug target pool larger than the {MAX_IMAGE_ARRAY_ELEMENTS} slots a shader declares.",
			images.len()
		);
		let mut elements = [None; MAX_IMAGE_ARRAY_ELEMENTS];
		for (element, image) in elements.iter_mut().zip(images) {
			*element = Some(*image);
		}
		Self::ImageArray(name, elements)
	}

	pub fn combined_image_sampler(
		name: &'static str,
		image: impl Into<ghi::BaseImageHandle>,
		sampler: ghi::SamplerHandle,
		layout: ghi::Layouts,
	) -> Self {
		Self::CombinedImageSampler(name, image.into(), sampler, layout)
	}

	fn name(&self) -> &str {
		match self {
			Self::Buffer(name, ..)
			| Self::PlannedBuffer(name, ..)
			| Self::Image(name, ..)
			| Self::ImageArray(name, ..)
			| Self::CombinedImageSampler(name, ..) => name,
		}
	}

	/// Whether this resource can be bound to a binding of `kind` with `count` elements: an image array only to an
	/// arrayed storage image it fits in, and a single resource only to a binding of one element.
	fn matches(&self, kind: BindingKind, count: u32) -> bool {
		match (kind, self) {
			(BindingKind::StorageImage, Self::ImageArray(_, images)) => {
				count > 1 && images.iter().flatten().count() <= count as usize
			}
			(BindingKind::StorageBuffer, Self::Buffer(..) | Self::PlannedBuffer(..))
			| (BindingKind::StorageImage, Self::Image(..))
			| (BindingKind::CombinedImageSampler { .. }, Self::CombinedImageSampler(..)) => count == 1,
			_ => false,
		}
	}

	/// Adds the writes that bind this resource to `slot` of `set`: one per bound array element.
	fn descriptor_writes(
		&self,
		set: ghi::DescriptorSetHandle,
		slot: ghi::ResourceSlot,
		writes: &mut SmallVec<[ghi::DescriptorWrite; 8]>,
	) {
		match *self {
			Self::Buffer(_, buffer) | Self::PlannedBuffer(_, buffer) => {
				writes.push(ghi::DescriptorWrite::buffer(set, slot, buffer));
			}
			Self::Image(_, ghi::ImageOrSwapchain::Image(image)) => {
				writes.push(ghi::DescriptorWrite::image(set, slot, image, ghi::Layouts::General));
			}
			Self::Image(_, ghi::ImageOrSwapchain::Swapchain(swapchain)) => {
				writes.push(ghi::DescriptorWrite::swapchain(set, slot, swapchain));
			}
			Self::ImageArray(_, images) => {
				for (element, image) in images.iter().enumerate() {
					if let Some(image) = image {
						writes.push(
							ghi::DescriptorWrite::image(set, slot, *image, ghi::Layouts::General)
								.with_array_element(element as u32),
						);
					}
				}
			}
			Self::CombinedImageSampler(_, image, sampler, layout) => {
				writes.push(ghi::DescriptorWrite::combined_image_sampler(
					set, slot, image, sampler, layout,
				));
			}
		}
	}

	fn is_planned(&self) -> bool {
		matches!(self, Self::PlannedBuffer(..))
	}
}

/// Returns the bindings the shader reaches, across every descriptor set layout it declares.
fn used_bindings(sets: &[SetLayout]) -> impl Iterator<Item = &Binding> {
	sets.iter().flat_map(|set| {
		let interface = set.interface();
		interface
			.bindings
			.iter()
			.enumerate()
			.filter(|(index, _)| interface.is_used(*index))
			.map(|(_, binding)| binding)
	})
}

/// Returns every binding the shader's layouts declare, whether or not the shader reaches it.
fn declared_bindings(sets: &[SetLayout]) -> impl Iterator<Item = &Binding> {
	sets.iter().flat_map(|set| &set.interface().bindings)
}

fn validate_binding_schema(sets: &[SetLayout]) -> Result<(), &'static str> {
	for binding in used_bindings(sets) {
		// Only storage images come arrayed, through [`Resource::image_array`].
		if binding.count != 1 && binding.kind != BindingKind::StorageImage {
			return Err(
				"Descriptor arrays are unsupported in simple compute passes. The most likely cause is that the BESL shader requires multiple resources for one binding.",
			);
		}
		if !binding.read && !binding.write {
			return Err(
				"Inaccessible binding in simple compute pass. The most likely cause is that a BESL binding declares neither read nor write access.",
			);
		}
		if matches!(binding.kind, BindingKind::CombinedImageSampler { .. }) && (!binding.read || binding.write) {
			return Err(
				"Sampled texture access is invalid in a simple compute pass. The most likely cause is that a combined image sampler was declared writable.",
			);
		}
	}
	for (index, binding) in declared_bindings(sets).enumerate() {
		if declared_bindings(sets)
			.take(index)
			.any(|previous| previous.name == binding.name)
		{
			return Err("Duplicate BESL binding name. The most likely cause is that two descriptor slots use the same symbol.");
		}
	}
	Ok(())
}

/// Checks the named resources against the pipeline's layouts: every binding the shader reaches takes exactly one
/// resource of its kind, and every resource names a binding one of the layouts declares.
///
/// A binding the shader does not reach may be left without a resource, which lets shaders that share a layout layer
/// bind only what each one reads.
fn validate_resources(sets: &[SetLayout], resources: &[Resource]) -> Result<(), String> {
	if let Some(resource) = resources
		.iter()
		.find(|resource| !resource.is_planned() && !declared_bindings(sets).any(|binding| binding.name == resource.name()))
	{
		return Err(format!(
			"Unknown compute resource `{}`. The most likely cause is that the resource name does not match a BESL binding of the shader or of a layout layer it lists.",
			resource.name()
		));
	}
	for binding in declared_bindings(sets) {
		let mut matches = resources.iter().filter(|resource| resource.name() == binding.name);
		let Some(resource) = matches.next() else {
			if used_bindings(sets).any(|used| used.name == binding.name) {
				return Err(format!(
					"Missing compute resource `{}`. The most likely cause is that the caller did not bind every BESL descriptor the shader reaches.",
					binding.name
				));
			}
			continue;
		};
		if matches.next().is_some() {
			return Err(format!(
				"Duplicate compute resource `{}`. The most likely cause is that the same BESL symbol was bound twice.",
				binding.name
			));
		}
		if !resource.matches(binding.kind, binding.count) {
			return Err(format!(
				"Compute resource `{}` has the wrong type. The most likely cause is that an image, sampler, or buffer was bound to an incompatible BESL binding.",
				binding.name
			));
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use resource_management::resources::material::{Binding, BindingKind, SetInterface, TextureView};

	use super::{Resource, SetLayout, validate_binding_schema, validate_resources};

	/// Builds the layouts of a shader that reaches every binding of its one layout.
	fn layouts(bindings: &[Binding]) -> [SetLayout; 1] {
		[SetLayout::new(&SetInterface::with_bindings("test.besl", bindings.to_vec()))]
	}

	fn binding(name: &str, kind: BindingKind, slot: u32, read: bool, write: bool) -> Binding {
		Binding {
			name: name.to_string(),
			kind,
			count: 1,
			slot,
			buffer_stride: matches!(kind, BindingKind::StorageBuffer).then_some(4),
			read,
			write,
		}
	}

	/// An arrayed storage image takes an image array that fits it, and nothing else; a single binding takes no array.
	#[test]
	fn image_arrays_bind_only_arrayed_storage_images_they_fit() {
		let mut device = ghi::debug::Device::new();
		let image: ghi::BaseImageHandle = device
			.build_dynamic_image(ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Storage))
			.into();
		let array = Resource::image_array("pool", &[image, image]);
		let single = Resource::image("pool", image);

		assert!(array.matches(BindingKind::StorageImage, 15));
		assert!(array.matches(BindingKind::StorageImage, 2));
		assert!(!array.matches(BindingKind::StorageImage, 1));
		assert!(!single.matches(BindingKind::StorageImage, 15));
		assert!(single.matches(BindingKind::StorageImage, 1));
		assert!(!array.matches(BindingKind::StorageBuffer, 15));

		let arrayed = layouts(&[Binding {
			count: 15,
			..binding("pool", BindingKind::StorageImage, 2, false, true)
		}]);
		assert!(validate_binding_schema(&arrayed).is_ok());
		assert!(validate_resources(&arrayed, &[array]).is_ok());
		assert!(validate_resources(&arrayed, &[single]).is_err());
	}

	#[test]
	fn named_resources_validate_complete_type_safe_order_independent_sets() {
		let bindings = [
			binding("parameters", BindingKind::StorageBuffer, 0, true, false),
			binding(
				"source",
				BindingKind::CombinedImageSampler {
					view: TextureView::Texture2D,
				},
				1,
				true,
				false,
			),
			binding("result", BindingKind::StorageImage, 2, false, true),
		];
		let mut device = ghi::debug::Device::new();
		let buffer = device.create_acceleration_structure_instance_buffer(None, 1);
		let image = device.build_dynamic_image(ghi::image::Builder::new(
			ghi::Formats::RGBA16F,
			ghi::Uses::Image | ghi::Uses::Storage,
		));
		let sampler = device.build_sampler(ghi::sampler::Builder::new());
		let resources = [
			Resource::image("result", image),
			Resource::buffer("parameters", buffer),
			Resource::combined_image_sampler("source", image, sampler, ghi::Layouts::Read),
		];
		validate_resources(&layouts(&bindings), &resources).expect("complete resources should validate");

		assert!(bindings[0].read && !bindings[0].write);
		let mut array = bindings[0].clone();
		array.count = 2;

		assert!(validate_binding_schema(&layouts(&[array])).is_err());
		let mut writable_sampler = bindings[1].clone();
		writable_sampler.write = true;

		assert!(validate_binding_schema(&layouts(&[writable_sampler])).is_err());
		assert!(
			validate_resources(&layouts(&bindings), &resources[..2])
				.expect_err("Expected a missing resource")
				.starts_with("Missing compute resource `source`.")
		);
		let duplicate = [resources[0], resources[1], resources[2], resources[2]];

		assert!(
			validate_resources(&layouts(&bindings), &duplicate)
				.expect_err("Expected a duplicate resource")
				.starts_with("Duplicate compute resource `source`.")
		);
		let wrong = [resources[0], resources[2], Resource::image("parameters", image)];

		assert!(
			validate_resources(&layouts(&bindings), &wrong)
				.expect_err("Expected a resource type mismatch")
				.starts_with("Compute resource `parameters` has the wrong type.")
		);
		let unknown = [resources[0], resources[1], resources[2], Resource::image("typo", image)];

		assert!(
			validate_resources(&layouts(&bindings), &unknown)
				.expect_err("Expected an unknown resource")
				.starts_with("Unknown compute resource `typo`.")
		);
		let planned = [
			resources[0],
			resources[1],
			resources[2],
			Resource::planned_buffer("future", buffer),
		];
		validate_resources(&layouts(&bindings), &planned).expect("planned resources should validate");
	}
}
