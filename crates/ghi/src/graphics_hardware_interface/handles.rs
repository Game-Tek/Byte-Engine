//! Backend-independent GHI handles.

// HANDLES

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct QueueHandle(pub(crate) u64);

/// The `BaseBufferHandle` struct identifies a static buffer without exposing its element type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug, PartialOrd, Ord)]
pub struct BaseBufferHandle(pub(crate) u64);

impl MasterHandle for BaseBufferHandle {
	fn new(i: u64) -> Self {
		BaseBufferHandle(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

/// The `BufferHandle` struct identifies a static buffer together with the type of its contents.
///
/// `T` is either one [`bytemuck::Pod`] value or a slice `[E]` whose length is chosen at creation with
/// [`crate::buffer::Builder::length`]. See [`crate::buffer::BufferContents`].
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
pub struct DynamicBufferHandle<T>(pub(crate) BaseBufferHandle, pub(crate) std::marker::PhantomData<T>);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BaseImageHandle(pub(crate) u64);

impl MasterHandle for BaseImageHandle {
	fn new(i: u64) -> Self {
		BaseImageHandle(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

impl From<BaseImageHandle> for Handles {
	fn from(value: BaseImageHandle) -> Self {
		Handles::Image(ImageHandle(value))
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageHandle(pub(crate) BaseImageHandle);

impl From<ImageHandle> for BaseImageHandle {
	fn from(value: ImageHandle) -> Self {
		value.0
	}
}

/// The `ImageGroupHandle` struct identifies images that may share device memory when their lifetimes do not overlap.
///
/// Create a group with [`crate::context::ContextCreate::create_image_group`], add images to it with
/// [`crate::image::Builder::group`], then give the members memory with
/// [`crate::frame::Frame::place_image_group`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageGroupHandle(pub(crate) u64);

/// The `DynamicImageHandle` struct addresses a frame-local image that can be written independently for each frame in flight.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct DynamicImageHandle(pub(crate) BaseImageHandle);

impl From<DynamicImageHandle> for BaseImageHandle {
	fn from(value: DynamicImageHandle) -> Self {
		value.0
	}
}

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct TopLevelAccelerationStructureHandle(pub(crate) u64);

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BottomLevelAccelerationStructureHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CommandBufferHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ShaderHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PipelineHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MeshHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SynchronizerHandle(pub(crate) u64);

impl MasterHandle for SynchronizerHandle {
	fn new(i: u64) -> Self {
		Self(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// The `DescriptorSetHandle` struct identifies a retained group of flat shader resource writes.
pub struct DescriptorSetHandle(pub(crate) u64);

/// The `PipelineLayoutHandle` struct identifies a pipeline resource layout.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineLayoutHandle(pub(crate) u64);

/// The `SamplerHandle` struct identifies an image sampler.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SamplerHandle(pub(crate) u64);

/// The `SwapchainHandle` struct identifies a presentation swapchain.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwapchainHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AllocationHandle(pub(crate) u64);

/// The `TextureCopyHandle` struct identifies one texture-transfer invocation within its creating context.
///
/// Handle values can overlap across contexts, so pass a handle only to the [`crate::Context`] that created it.
/// Submit the command that returned the handle, then pass it once to [`crate::Context::get_image_data`]. Successful
/// mapping consumes the handle value and releases backend staging; later mapping attempts return
/// [`crate::TextureTransferError::InvalidHandle`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextureCopyHandle(pub(crate) u64);

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

impl From<DynamicImageHandle> for Handles {
	fn from(val: DynamicImageHandle) -> Self {
		val.0.into()
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Handles {
	Buffer(BaseBufferHandle),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
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
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
}

impl From<BaseBufferHandle> for Handles {
	fn from(val: BaseBufferHandle) -> Self {
		Handles::Buffer(val)
	}
}

impl From<ImageHandle> for Handles {
	fn from(val: ImageHandle) -> Self {
		Handles::Image(val)
	}
}

impl From<SynchronizerHandle> for Handles {
	fn from(val: SynchronizerHandle) -> Self {
		Handles::Synchronizer(val)
	}
}

pub(crate) trait MasterHandle: Sized + Copy {
	fn new(i: u64) -> Self;
	fn index(&self) -> u64;
}

impl<T: ?Sized> MasterHandle for BufferHandle<T> {
	fn new(i: u64) -> Self {
		Self(BaseBufferHandle(i), std::marker::PhantomData)
	}

	fn index(&self) -> u64 {
		self.0.0
	}
}

pub(crate) trait PrivateHandle: Copy {
	fn new(i: u64) -> Self;
	fn index(&self) -> u64;
}
