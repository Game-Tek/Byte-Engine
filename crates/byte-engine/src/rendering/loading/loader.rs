//! The loading timeline: one context and one copy queue that batch every lane's uploads.

use std::{
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	},
	time::{Duration, Instant},
};

use ghi::{
	command_buffer::CommandBufferRecording as _,
	context::{Context as _, ContextCreate as _},
	io::{ResourceIoContext as _, ResourceIoQueue as _, ResourceIoTicket as _},
	queue::Queue as _,
};
use smallvec::SmallVec;
use utils::Extent;

use crate::{
	application::parameters::Parameters,
	rendering::{
		GraphicsDevice,
		resource_loading::{StagingLease, UploadStagingArena, UploadStagingWorker},
	},
};

/// How long a batch collects uploads before the loader submits it, unless staging runs out first.
const DEFAULT_BATCH_WINDOW: Duration = Duration::from_millis(4);
/// How often the loader checks the batch on the copy queue while uploads are pending or in flight.
const POLL_INTERVAL: Duration = Duration::from_micros(500);
/// The staging capacity that every lane of every pipeline shares.
const STAGING_BYTE_COUNT: usize = 32 * 1024 * 1024;

/// The `Loader` struct owns the loading timeline: its context, its copy queue, and the staging memory lanes share.
///
/// Lanes hand it uploads as plain data. It collects them into one batch until the batch window elapses, staging
/// runs out, or every busy lane is waiting on it, submits the batch on the copy queue, and keeps collecting the next
/// batch while the GPU copies. One batch is in flight at a time: the next one waits until the copy queue is free
/// again. Finished images leave the loader context as [`ghi::implementation::DetachedImage`] values. Lanes report
/// them to the render thread, which interns them when it adopts the notification. The render thread never waits on
/// the loader.
///
/// Create it with [`Self::new`], import every render buffer that lanes append to with [`Self::import_buffer`], and
/// create lanes with [`spawn`](super::spawn). Next, start it with [`Self::run`] on the thread that runs the lanes.
/// See the [loading timeline design](/docs/develop/rendering/loading) for the batching and staging rules.
pub struct Loader {
	pub(super) batcher: Batcher,
	staging_worker: UploadStagingWorker,
	/// Cloned into every lane, and dropped when the loader starts so the channel closes once the last lane is gone.
	pub(super) uploads: kanal::AsyncSender<UploadRequest>,
}

impl Loader {
	/// Creates the loader context on `device`'s copy queue and the staging memory lanes share.
	///
	/// # Parameters
	/// - `render.loading.batch-window`: Milliseconds a batch collects uploads before the loader submits it.
	///   Defaults to 4.
	pub fn new(device: &GraphicsDevice, parameters: &dyn Parameters) -> Self {
		let mut context = device.create_context();
		let (staging_buffer, staging, staging_worker) =
			UploadStagingArena::create(&mut context, STAGING_BYTE_COUNT, "Loader Staging Buffer");
		let command_buffer = context.queue(device.copy_queue()).create_command_buffer(Some("Loader Batch"));
		let synchronizer = context.create_synchronizer(Some("Loader Batch"), false);
		// Native I/O stays optional so devices without it still load staged textures.
		let io_queue = context
			.create_resource_io_queue(ghi::io::ResourceIoQueueDescriptor::new().name("Loader I/O"))
			.ok();
		let batch_window = parameters
			.get_parameter("render.loading.batch-window")
			.and_then(|parameter| parameter.value().parse::<f64>().ok())
			.filter(|milliseconds| milliseconds.is_finite() && *milliseconds >= 0.0)
			.map_or(DEFAULT_BATCH_WINDOW, |milliseconds| {
				Duration::from_secs_f64(milliseconds / 1000.0)
			});
		let (uploads, requests) = kanal::unbounded_async();

		Self {
			batcher: Batcher {
				context,
				command_buffer,
				synchronizer,
				staging_buffer,
				staging,
				io_queue,
				requests,
				busy_lanes: Arc::new(AtomicUsize::new(0)),
				batch_window,
				pending: Vec::new(),
				in_flight: Vec::new(),
				image_copies: Vec::new(),
				buffer_copies: Vec::new(),
			},
			staging_worker,
			uploads,
		}
	}

	/// Imports a buffer the render context shared, so lanes can append to it with [`BufferRegion`] copies.
	///
	/// Next, pass the returned handle to the lanes that write the buffer.
	pub fn import_buffer<T: ?Sized>(&mut self, buffer: ghi::implementation::SharedBuffer<T>) -> ghi::BufferHandle<T> {
		self.batcher.context.import_buffer(buffer)
	}

	/// Starts the loader's tasks on `runtime`.
	///
	/// Run it on the thread that runs the lanes. The loader stops once every lane is gone and its last batch is
	/// finished.
	pub fn run(self, runtime: &compio::runtime::Runtime) {
		let Self {
			batcher,
			staging_worker,
			uploads,
		} = self;
		drop(uploads);
		runtime.spawn(staging_worker.run()).detach();
		runtime.spawn(batcher.run()).detach();
	}
}

/// The `ImageDescription` struct names and shapes one image the loader creates for an upload.
///
/// Loader images live in device memory, never change size, and are sampled once their upload completes.
pub struct ImageDescription {
	pub name: String,
	pub format: ghi::Formats,
	pub extent: Extent,
	pub mip_levels: u32,
	/// Whether the image has six layers that shaders sample as a cube.
	pub cube: bool,
}

impl ImageDescription {
	/// Returns the GHI builder for this image. The builder's defaults already make it static and device-only.
	fn builder(&self) -> ghi::image::Builder<'_> {
		let builder = ghi::image::Builder::new(self.format, ghi::Uses::Image | ghi::Uses::TransferDestination)
			.name(&self.name)
			.extent(self.extent)
			.mip_levels(self.mip_levels);
		if self.cube { builder.cube_compatible() } else { builder }
	}
}

/// The `ImageRegion` struct places one mip of an image upload inside its staging lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageRegion {
	/// The byte offset from the start of the staging lease.
	pub offset: usize,
	pub bytes_per_row: usize,
	pub bytes_per_image: usize,
	pub mip_level: u32,
}

/// The `ImageUpload` struct pairs one image with the staged mips that fill it.
pub struct ImageUpload {
	pub description: ImageDescription,
	pub regions: SmallVec<[ImageRegion; 16]>,
}

/// The `BufferRegion` struct copies one staged byte range into a render buffer the loader imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferRegion {
	/// The byte offset from the start of the staging lease.
	pub offset: usize,
	/// A handle returned by [`Loader::import_buffer`].
	pub destination: ghi::BaseBufferHandle,
	pub destination_offset: usize,
	pub size: usize,
}

/// The `NativeImageUpload` struct fills one image straight from a file through the device's native I/O queue.
pub struct NativeImageUpload {
	pub description: ImageDescription,
	pub path: PathBuf,
	pub compression: ghi::io::ResourceIoCompression,
	pub regions: SmallVec<[NativeImageRegion; 16]>,
}

/// The `NativeImageRegion` struct locates one decoded mip inside a native file.
#[derive(Clone, Copy, Debug)]
pub struct NativeImageRegion {
	/// The byte offset of the mip in the file's decoded stream.
	pub file_offset: usize,
	pub mip_level: u32,
	pub extent: Extent,
	pub bytes_per_row: usize,
	pub bytes_per_image: usize,
}

/// The `UploadRequest` enum carries one lane's upload and the channel its result goes back through.
pub(super) enum UploadRequest {
	/// Creates images and fills them, and any imported buffers, from one staging lease.
	Staged {
		staging: StagingLease,
		images: SmallVec<[ImageUpload; 2]>,
		buffers: SmallVec<[BufferRegion; 16]>,
		reply: kanal::Sender<SmallVec<[ghi::implementation::DetachedImage; 2]>>,
	},
	NativeImage {
		upload: NativeImageUpload,
		reply: kanal::Sender<Result<ghi::implementation::DetachedImage, String>>,
	},
}

/// The `Completion` enum keeps what one upload needs until its batch finishes on the GPU.
enum Completion {
	Staged {
		images: SmallVec<[ghi::ImageHandle; 2]>,
		/// Returns to the arena once the copies that read it finished.
		staging: StagingLease,
		reply: kanal::Sender<SmallVec<[ghi::implementation::DetachedImage; 2]>>,
	},
	NativeImage {
		image: ghi::ImageHandle,
		ticket: ghi::implementation::ResourceIoTicket,
		reply: kanal::Sender<Result<ghi::implementation::DetachedImage, String>>,
	},
}

/// The `Batcher` struct is the running half of a [`Loader`]: it collects, submits, and finishes batches.
pub(super) struct Batcher {
	context: ghi::implementation::Context,
	command_buffer: ghi::CommandBufferHandle,
	synchronizer: ghi::SynchronizerHandle,
	staging_buffer: ghi::BaseBufferHandle,
	pub(super) staging: Arc<UploadStagingArena>,
	io_queue: Option<ghi::implementation::ResourceIoQueue>,
	requests: kanal::AsyncReceiver<UploadRequest>,
	/// How many lanes are loading a resource right now, across every pipeline.
	pub(super) busy_lanes: Arc<AtomicUsize>,
	batch_window: Duration,
	/// Uploads collected for the next batch.
	pending: Vec<UploadRequest>,
	/// What the batch on the copy queue needs once it finishes. Empty while the queue is free.
	in_flight: Vec<Completion>,
	/// Copy lists rebuilt for every batch. They keep their storage so collecting a batch allocates nothing.
	image_copies: Vec<ghi::BufferImageCopyDescriptor>,
	buffer_copies: Vec<ghi::BufferCopyDescriptor>,
}

impl Batcher {
	/// Collects uploads into batches and submits them on the copy queue until every lane is gone.
	///
	/// A batch is due once its window elapsed, staging ran out, or every busy lane already waits on an upload, since
	/// waiting then would only delay the lanes. Once every lane is gone none is busy, so the last batch is due at
	/// once. A batch is submitted only when the previous one finished. While work is pending or in flight, the loop
	/// checks it every [`POLL_INTERVAL`] without blocking, so lanes on this thread keep reading and decoding.
	async fn run(mut self) {
		let mut opened = Instant::now();
		loop {
			if self.pending.is_empty() && self.in_flight.is_empty() {
				// Nothing is left to check, so sleep until a lane uploads.
				let Ok(request) = self.requests.recv().await else {
					return;
				};
				self.pending.push(request);
				opened = Instant::now();
			}
			while let Ok(Some(request)) = self.requests.as_sync().try_recv() {
				if self.pending.is_empty() {
					opened = Instant::now();
				}
				self.pending.push(request);
			}

			if !self.in_flight.is_empty() && self.is_finished() {
				self.finish();
			}
			// Each busy lane waits on at most one upload of its own, so this many pending uploads means none can join.
			let every_lane_waits = self.pending.len() >= self.busy_lanes.load(Ordering::Acquire);
			let due = opened.elapsed() >= self.batch_window || self.staging.is_exhausted() || every_lane_waits;
			if self.in_flight.is_empty() && !self.pending.is_empty() && due {
				self.dispatch();
			}

			if !self.pending.is_empty() || !self.in_flight.is_empty() {
				compio::time::sleep(POLL_INTERVAL).await;
			}
		}
	}

	/// Creates every image of the pending batch, records all of its copies in one command buffer, and submits it.
	fn dispatch(&mut self) {
		let mut pending = std::mem::take(&mut self.pending);
		for request in pending.drain(..) {
			match request {
				UploadRequest::Staged {
					staging,
					images,
					buffers,
					reply,
				} => {
					let base = staging.offset();
					let mut handles = SmallVec::new();
					for upload in &images {
						let image = self.context.build_image(upload.description.builder());
						self.image_copies.extend(upload.regions.iter().map(|region| {
							ghi::BufferImageCopyDescriptor::new(
								self.staging_buffer,
								base + region.offset,
								region.bytes_per_row,
								region.bytes_per_image,
								image.into(),
								region.mip_level,
							)
						}));
						handles.push(image);
					}
					self.buffer_copies.extend(buffers.iter().map(|region| {
						ghi::BufferCopyDescriptor::new(
							self.staging_buffer,
							base + region.offset,
							region.destination,
							region.destination_offset,
							region.size,
						)
					}));
					self.in_flight.push(Completion::Staged {
						images: handles,
						staging,
						reply,
					});
				}
				UploadRequest::NativeImage { upload, reply } => match self.submit_native(upload) {
					Ok((image, ticket)) => self.in_flight.push(Completion::NativeImage { image, ticket, reply }),
					Err(error) => {
						let _ = reply.send(Err(error));
					}
				},
			}
		}
		self.pending = pending;

		if !self.image_copies.is_empty() || !self.buffer_copies.is_empty() {
			let mut recording = self.context.create_command_buffer_recording(self.command_buffer);
			recording.copy_buffers(&self.buffer_copies);
			recording.copy_buffer_to_images(&self.image_copies);
			recording.execute(self.synchronizer);
		}
		self.image_copies.clear();
		self.buffer_copies.clear();
	}

	/// Creates the image for a native upload, opens its file, and submits one read per mip on the native I/O queue.
	fn submit_native(
		&mut self,
		upload: NativeImageUpload,
	) -> Result<(ghi::ImageHandle, ghi::implementation::ResourceIoTicket), String> {
		let name = upload.description.name.as_str();
		let queue = self.io_queue.as_mut().ok_or_else(|| {
			format!(
				"Texture native I/O is unavailable for {name}. The most likely cause is that the device has no native storage queue."
			)
		})?;
		let file = queue
			.open_file(
				ghi::io::ResourceIoFileDescriptor::new(&upload.path)
					.compression(upload.compression)
					.name(name),
			)
			.map_err(|error| {
				format!(
					"Texture I/O file could not be opened for {name}. The most likely cause is missing or unreadable native backing storage. {error}"
				)
			})?;
		let image = self.context.build_image(upload.description.builder());
		let requests = upload
			.regions
			.iter()
			.map(|region| {
				ghi::io::ResourceIoImageLoad::new(
					ghi::io::ResourceIoFileRegion::new(file, region.file_offset),
					image,
					0,
					region.mip_level,
					region.extent,
					region.bytes_per_row,
					region.bytes_per_image,
				)
				.into()
			})
			.collect::<SmallVec<[ghi::io::ResourceIoRequest; 16]>>();
		match queue.submit(&self.context, Some(name), &requests) {
			Ok(ticket) => Ok((image, ticket)),
			Err(error) => {
				// Nothing will fill the image, so free it instead of leaving it in the loader context.
				drop(self.context.export_image(image));
				Err(format!(
					"Texture I/O submission failed for {name}. The most likely cause is an unsupported request or unavailable native queue. {error}"
				))
			}
		}
	}

	/// Returns whether the GPU finished every copy and native read of the batch in flight, without blocking.
	fn is_finished(&mut self) -> bool {
		self.context.poll_synchronizer(self.synchronizer)
			&& self.in_flight.iter().all(|completion| match completion {
				Completion::NativeImage { ticket, .. } => ticket.status() != ghi::io::ResourceIoStatus::Pending,
				Completion::Staged { .. } => true,
			})
	}

	/// Returns the finished batch's staging to the arena and hands every lane its result.
	///
	/// A lane that stopped waiting drops its images when its reply fails to send.
	fn finish(&mut self) {
		let mut in_flight = std::mem::take(&mut self.in_flight);
		for completion in in_flight.drain(..) {
			match completion {
				Completion::Staged { images, staging, reply } => {
					drop(staging);
					let images = images.into_iter().map(|image| self.context.export_image(image)).collect();
					let _ = reply.send(images);
				}
				Completion::NativeImage { image, ticket, reply } => {
					let image = self.context.export_image(image);
					let result = ticket.wait().map(|()| image).map_err(|error| {
						format!(
							"Texture I/O failed. The most likely cause is unreadable or incompatible compressed texture data. {error}"
						)
					});
					let _ = reply.send(result);
				}
			}
		}
		self.in_flight = in_flight;
	}
}
