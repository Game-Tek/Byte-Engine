//! Backend-independent GHI handles.

// HANDLES

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct QueueHandle;
}

crate::handle::index_handle! {
	/// The `BaseBufferHandle` struct identifies a static buffer without exposing its element type.
	#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug, PartialOrd, Ord)]
	pub struct BaseBufferHandle;
}

impl MasterHandle for BaseBufferHandle {
	fn new(i: u64) -> Self {
		BaseBufferHandle::from_index(i)
	}

	fn index(&self) -> u64 {
		BaseBufferHandle::index(*self)
	}
}

/// The `BufferHandle` struct identifies a static buffer together with the type of its contents.
///
/// `T` is either one [`bytemuck::Pod`] value or a slice `[E]` whose length is chosen at creation with
/// [`crate::buffer::Builder::length`]. See [`crate::buffer::BufferContents`].
#[repr(transparent)]
pub struct BufferHandle<T: ?Sized>(pub(crate) BaseBufferHandle, pub(crate) std::marker::PhantomData<T>);

// Manual impls keep handles copyable and comparable for slice contents, which derives would reject because `[E]`
// is neither `Clone` nor `Sized`.
impl<T: ?Sized> Clone for BufferHandle<T> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T: ?Sized> Copy for BufferHandle<T> {}

impl<T: ?Sized> PartialEq for BufferHandle<T> {
	fn eq(&self, other: &Self) -> bool {
		self.0 == other.0
	}
}

impl<T: ?Sized> Eq for BufferHandle<T> {}

impl<T: ?Sized> std::hash::Hash for BufferHandle<T> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.0.hash(state);
	}
}

impl<T: ?Sized> std::fmt::Debug for BufferHandle<T> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.debug_tuple("BufferHandle").field(&self.0).finish()
	}
}

/// The `DynamicBufferHandle` struct identifies a resizable buffer with its element type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
#[repr(transparent)]
pub struct DynamicBufferHandle<T>(pub(crate) BaseBufferHandle, pub(crate) std::marker::PhantomData<T>);

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct BaseImageHandle;
}

impl MasterHandle for BaseImageHandle {
	fn new(i: u64) -> Self {
		BaseImageHandle::from_index(i)
	}

	fn index(&self) -> u64 {
		BaseImageHandle::index(*self)
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct ImageHandle(pub(crate) BaseImageHandle);

impl From<ImageHandle> for BaseImageHandle {
	fn from(value: ImageHandle) -> Self {
		value.0
	}
}

crate::handle::index_handle! {
	/// The `ImageGroupHandle` struct identifies images that may share device memory when their lifetimes do not overlap.
	///
	/// Create a group with [`crate::context::ContextCreate::create_image_group`], add images to it with
	/// [`crate::image::Builder::group`], then give the members memory with
	/// [`crate::frame::Frame::place_image_group`].
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct ImageGroupHandle;
}

/// The `DynamicImageHandle` struct addresses a frame-local image that can be written independently for each frame in flight.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
#[repr(transparent)]
pub struct DynamicImageHandle(pub(crate) BaseImageHandle);

impl From<DynamicImageHandle> for BaseImageHandle {
	fn from(value: DynamicImageHandle) -> Self {
		value.0
	}
}

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
#[repr(transparent)]
pub struct TopLevelAccelerationStructureHandle(std::num::NonZeroU64);

impl TopLevelAccelerationStructureHandle {
	/// Retains the nonzero generation bits supplied by the resource slot.
	pub(crate) fn from_bits(bits: u64) -> Self {
		Self(
			std::num::NonZeroU64::new(bits).expect(
				"Resource handle is zero. Its generation bits are missing. Use a handle returned by resource creation.",
			),
		)
	}

	pub(crate) const fn bits(self) -> u64 {
		self.0.get()
	}
}

const _: () = assert!(size_of::<Option<TopLevelAccelerationStructureHandle>>() == size_of::<u64>());

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
#[repr(transparent)]
pub struct BottomLevelAccelerationStructureHandle(std::num::NonZeroU64);

impl BottomLevelAccelerationStructureHandle {
	/// Retains the nonzero generation bits supplied by the resource slot.
	pub(crate) fn from_bits(bits: u64) -> Self {
		Self(
			std::num::NonZeroU64::new(bits).expect(
				"Resource handle is zero. Its generation bits are missing. Use a handle returned by resource creation.",
			),
		)
	}

	pub(crate) const fn bits(self) -> u64 {
		self.0.get()
	}
}

const _: () = assert!(size_of::<Option<BottomLevelAccelerationStructureHandle>>() == size_of::<u64>());

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct CommandBufferHandle;
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct ShaderHandle;
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	pub struct PipelineHandle;
}

crate::handle::index_handle! {
	/// The `ShaderBindingTableHandle` struct names the table that maps the records rays hit to the hit groups of a ray
	/// tracing pipeline.
	///
	/// Create one with [`crate::context::ContextCreate::create_shader_binding_table`], fill it with
	/// [`crate::context::ContextCreate::write_shader_binding_table`], then pass it to
	/// [`crate::command_buffer::BoundRayTracingPipelineMode::trace_rays`].
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct ShaderBindingTableHandle;
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct MeshHandle;
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct SynchronizerHandle;
}

impl MasterHandle for SynchronizerHandle {
	fn new(i: u64) -> Self {
		Self::from_index(i)
	}

	fn index(&self) -> u64 {
		SynchronizerHandle::index(*self)
	}
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	/// The `DescriptorSetHandle` struct identifies a retained group of flat shader resource writes.
	pub struct DescriptorSetHandle;
}

impl MasterHandle for DescriptorSetHandle {
	fn new(i: u64) -> Self {
		Self::from_index(i)
	}

	fn index(&self) -> u64 {
		DescriptorSetHandle::index(*self)
	}
}

crate::handle::index_handle! {
	/// The `PipelineLayoutHandle` struct identifies a pipeline resource layout.
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct PipelineLayoutHandle;
}

crate::handle::index_handle! {
	/// The `SamplerHandle` struct identifies an image sampler.
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct SamplerHandle;
}

crate::handle::index_handle! {
	/// The `SwapchainHandle` struct identifies a presentation swapchain.
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct SwapchainHandle;
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct AllocationHandle;
}

crate::handle::index_handle! {
	/// The `CounterHandle` struct identifies a GPU timing counter of the context that created it.
	///
	/// Create one with [`crate::context::ContextCreate::create_counter`], bracket the frame work it measures with
	/// [`crate::command_buffer::CommonCommandBufferMode::counter`], and read the GPU time of the last completed frame
	/// with [`crate::context::Context::counter_duration`].
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct CounterHandle;
}

/// The `TextureCopyHandle` struct identifies one texture-transfer invocation within its creating context.
///
/// Handle values can overlap across contexts, so pass a handle only to the [`crate::Context`] that created it.
/// Submit the command that returned the handle, then pass it once to [`crate::Context::get_image_data`]. Successful
/// mapping consumes the handle value and releases backend staging; later mapping attempts return
/// [`crate::TextureTransferError::InvalidHandle`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct TextureCopyHandle(std::num::NonZeroU64);

impl TextureCopyHandle {
	/// Retains the nonzero generation bits supplied by the resource slot.
	pub(crate) fn from_bits(bits: u64) -> Self {
		Self(
			std::num::NonZeroU64::new(bits).expect(
				"Resource handle is zero. Its generation bits are missing. Use a handle returned by resource creation.",
			),
		)
	}

	pub(crate) const fn bits(self) -> u64 {
		self.0.get()
	}
}

const _: () = assert!(size_of::<Option<TextureCopyHandle>>() == size_of::<u64>());

/// The `Handles` enum lets the Vulkan backend key debug names by any public resource handle.
///
/// Only the Vulkan backend stores debug names this way, so the enum exists only on Linux.
#[cfg(target_os = "linux")]
#[cfg_attr(not(debug_assertions), allow(dead_code, reason = "Only debug builds keep debug names."))]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Handles {
	Buffer(BaseBufferHandle),
	CommandBuffer(CommandBufferHandle),
	Shader(ShaderHandle),
	Pipeline(PipelineHandle),
	Image(ImageHandle),
	Mesh(MeshHandle),
	Synchronizer(SynchronizerHandle),

	DescriptorSet(DescriptorSetHandle),
	PipelineLayout(PipelineLayoutHandle),
	Sampler(SamplerHandle),
	Swapchain(SwapchainHandle),
	Allocation(AllocationHandle),
	TextureCopy(TextureCopyHandle),
}

#[cfg(target_os = "linux")]
impl From<BaseBufferHandle> for Handles {
	fn from(val: BaseBufferHandle) -> Self {
		Handles::Buffer(val)
	}
}

#[cfg(target_os = "linux")]
impl From<ImageHandle> for Handles {
	fn from(val: ImageHandle) -> Self {
		Handles::Image(val)
	}
}

#[cfg(target_os = "linux")]
impl From<SynchronizerHandle> for Handles {
	fn from(val: SynchronizerHandle) -> Self {
		Handles::Synchronizer(val)
	}
}

impl<T: ?Sized> From<BufferHandle<T>> for BaseBufferHandle {
	fn from(val: BufferHandle<T>) -> Self {
		val.0
	}
}

impl<T: bytemuck::Pod> From<DynamicBufferHandle<T>> for BaseBufferHandle {
	fn from(val: DynamicBufferHandle<T>) -> Self {
		val.0
	}
}

pub(crate) trait MasterHandle: Sized + Copy {
	fn new(i: u64) -> Self;
	fn index(&self) -> u64;
}

impl<T: ?Sized> MasterHandle for BufferHandle<T> {
	fn new(i: u64) -> Self {
		Self(BaseBufferHandle::from_index(i), std::marker::PhantomData)
	}

	fn index(&self) -> u64 {
		self.0.index()
	}
}

pub(crate) trait PrivateHandle: Copy {
	fn new(i: u64) -> Self;
	fn index(&self) -> u64;
}

const _: () = {
	assert!(size_of::<Option<BufferHandle<[u32]>>>() == size_of::<u64>());
	assert!(size_of::<Option<DynamicBufferHandle<u32>>>() == size_of::<u64>());
	assert!(size_of::<Option<ImageHandle>>() == size_of::<u64>());
	assert!(size_of::<Option<DynamicImageHandle>>() == size_of::<u64>());
};
