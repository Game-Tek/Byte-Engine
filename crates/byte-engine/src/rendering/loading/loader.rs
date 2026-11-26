//! The loading timeline: one context and one copy queue that batch every lane's uploads.

#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]

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
/// How often the loader checks native reads while they run.
const POLL_INTERVAL: Duration = Duration::from_micros(500);
/// The staging capacity that every lane of every pipeline shares.
const STAGING_BYTE_COUNT: usize = 32 * 1024 * 1024;

/// The `Loader` struct owns the loading timeline: its context, its copy queue, and the staging memory lanes share.
///
/// Lanes hand it uploads as plain data and wait for its answer without blocking their thread. On a thread of its
/// own, it collects uploads into one batch until the batch window elapses, staging runs out, or every busy lane is
/// waiting on it, and submits the batch as one workload on the copy queue, which returns once the copies landed.
/// Then it answers each lane. Finished images leave the loader context as [`ghi::implementation::DetachedImage`]
/// values. Lanes report them to the render thread, which interns them when it adopts the notification. The render
/// thread never waits on the loader.
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
	/// Read from `parameters`, the `render.loading` subtree of the application's:
	/// - `batch-window`: Milliseconds a batch collects uploads before the loader submits it. Defaults to 4.
	pub fn new(device: &GraphicsDevice, parameters: impl Parameters) -> Self {
		let mut context = device.create_context();
		let (staging_buffer, staging, staging_worker) =
			UploadStagingArena::create(&mut context, STAGING_BYTE_COUNT, "Loader Staging Buffer");
		// Native I/O stays optional so devices without it still load staged textures.
		let io_queue = context
			.create_resource_io_queue(ghi::io::ResourceIoQueueDescriptor::new().name("Loader I/O"))
			.inspect_err(|error| {
				log::warn!(
					"Native texture I/O is unavailable, so GPU-encoded textures won't load. The most likely cause is that this platform's storage runtime is missing. {error}"
				);
			})
			.ok();
		let batch_window = parameters
			.get_parameter("batch-window")
			.and_then(|parameter| parameter.parse::<f64>().ok())
			.filter(|milliseconds| milliseconds.is_finite() && *milliseconds >= 0.0)
			.map_or(DEFAULT_BATCH_WINDOW, |milliseconds| {
				Duration::from_secs_f64(milliseconds / 1000.0)
			});
		let (uploads, requests) = kanal::unbounded_async();

		Self {
			batcher: Batcher {
				context,
				queue: device.copy_queue(),
				staging_buffer,
				staging,
				io_queue,
				requests,
				busy_lanes: Arc::new(AtomicUsize::new(0)),
				batch_window,
				pending: Vec::new(),
				reads: Vec::new(),
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

	/// Starts the staging worker on `runtime` and the batcher on a thread of its own.
	///
	/// Run it on the thread that runs the lanes. The batcher waits for the GPU at every batch, so it never shares that
	/// thread. The loader stops once every lane is gone and its last batch is finished.
	pub fn run(self, runtime: &compio::runtime::Runtime) {
		let Self {
			batcher,
			staging_worker,
			uploads,
		} = self;
		drop(uploads);
		runtime.spawn(staging_worker.run()).detach();
		std::thread::Builder::new()
			.name("Loader Batcher".to_string())
			.spawn(move || batcher.run())
			.expect("Failed to start the loader batcher thread. The most likely cause is that the process ran out of threads. Free some threads, or raise the process's thread limit.");
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
	///
	/// Loaded images are transfer sources too, so tests and debugging tools can read back what a load uploaded.
	fn builder(&self) -> ghi::image::Builder<'_> {
		let builder = ghi::image::Builder::new(
			self.format,
			ghi::Uses::Image | ghi::Uses::TransferDestination | ghi::Uses::TransferSource,
		)
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

/// The `NativeImageRegion` struct locates one band of rows of a mip inside a native file.
///
/// Texture loading creates one per mip for a container the GPU API addresses by decoded offset, or one per stored
/// block for a block-addressed payload. The loader asks the device for the source pitches, so the region carries only
/// where the bytes are and which texels they fill.
#[derive(Clone, Copy, Debug)]
pub struct NativeImageRegion {
	/// The byte offset of the region in the file's decoded stream.
	pub file_offset: usize,
	/// The physical offset and size of the region's independent block, for APIs that load blocks.
	pub stored: Option<(u64, usize)>,
	pub mip_level: u32,
	/// The first texel row of the mip the region fills.
	pub first_row: u32,
	/// The texels the region fills, starting at `first_row`.
	pub extent: Extent,
}

/// The `UploadRequest` enum carries one lane's upload and the channel its result goes back through.
pub(super) enum UploadRequest {
	/// Creates images and fills them, and any imported buffers, from one staging lease.
	Staged(StagedUpload),
	NativeImage {
		upload: NativeImageUpload,
		reply: kanal::Sender<Result<ghi::implementation::DetachedImage, String>>,
	},
}

/// The `StagedUpload` struct is one lane's upload from a staging lease, which waits in a batch until its copies land.
pub(super) struct StagedUpload {
	pub(super) staging: StagingLease,
	pub(super) images: SmallVec<[ImageUpload; 2]>,
	pub(super) buffers: SmallVec<[BufferRegion; 16]>,
	pub(super) reply: kanal::Sender<SmallVec<[ghi::implementation::DetachedImage; 2]>>,
}

/// The `NativeRead` struct keeps what one submitted native image upload owns until its ticket is terminal.
struct NativeRead {
	image: ghi::ImageHandle,
	/// The file the reads come from, closed once they finish so open files don't pile up past the process's limit.
	file: ghi::io::ResourceIoFileHandle,
	ticket: ghi::implementation::ResourceIoTicket,
	/// The image's name, which reports name the texture by.
	name: String,
	reply: kanal::Sender<Result<ghi::implementation::DetachedImage, String>>,
}

/// The `Batcher` struct is the running half of a [`Loader`]: it collects uploads, submits them in batches, and
/// answers the lanes once they landed.
pub(super) struct Batcher {
	context: ghi::implementation::Context,
	/// The copy queue every batch runs on.
	queue: ghi::QueueHandle,
	staging_buffer: ghi::BaseBufferHandle,
	pub(super) staging: Arc<UploadStagingArena>,
	io_queue: Option<ghi::implementation::ResourceIoQueue>,
	requests: kanal::AsyncReceiver<UploadRequest>,
	/// How many lanes are loading a resource right now, across every pipeline.
	pub(super) busy_lanes: Arc<AtomicUsize>,
	batch_window: Duration,
	/// Staged uploads collected for the next batch.
	pending: Vec<StagedUpload>,
	/// Native image reads that haven't finished.
	reads: Vec<NativeRead>,
	/// Copy lists rebuilt for every batch. They keep their storage so collecting a batch allocates nothing.
	image_copies: Vec<ghi::BufferImageCopyDescriptor>,
	buffer_copies: Vec<ghi::BufferCopyDescriptor>,
}

impl Batcher {
	/// Collects uploads into batches and submits them on the copy queue until every lane is gone.
	///
	/// A batch is due once its window elapsed, staging ran out, or every busy lane already waits on an upload, since
	/// waiting then would only delay the lanes. Once every lane is gone none is busy, so the last batch is due at
	/// once. Submitting a batch returns once its copies landed, so the next batch collects while lanes wait on this
	/// thread's answer, not while the GPU copies.
	fn run(mut self) {
		let requests = self.requests.clone();
		let requests = requests.as_sync();
		let mut opened = Instant::now();
		let mut open = true;
		loop {
			// Sleep until a lane uploads while nothing waits; otherwise check again by the next deadline.
			let request = if self.pending.is_empty() && self.reads.is_empty() {
				// Once every lane is gone the channel is closed, so this returns at once.
				let Ok(request) = requests.recv() else {
					return;
				};
				Some(request)
			} else {
				// Native reads finish on their own, so they're checked every poll interval.
				let window_left = self.batch_window.saturating_sub(opened.elapsed());
				let timeout = match (self.pending.is_empty(), self.reads.is_empty()) {
					(true, _) => POLL_INTERVAL,
					(false, true) => window_left,
					(false, false) => window_left.min(POLL_INTERVAL),
				};
				if !open {
					// No lane is left to send anything, and a closed channel returns at once; only native reads still run.
					std::thread::sleep(timeout);
				}
				match requests.recv_timeout(timeout) {
					Ok(request) => Some(request),
					Err(kanal::ReceiveErrorTimeout::Timeout) => None,
					Err(_) => {
						open = false;
						None
					}
				}
			};
			for request in request
				.into_iter()
				.chain(std::iter::from_fn(|| requests.try_recv().ok().flatten()))
			{
				if self.pending.is_empty() {
					opened = Instant::now();
				}
				self.accept(request);
			}

			self.finish_reads();
			// Each busy lane waits on at most one upload of its own, so this many uploads means none can join.
			let every_lane_waits = self.pending.len() + self.reads.len() >= self.busy_lanes.load(Ordering::Acquire);
			let due = opened.elapsed() >= self.batch_window || self.staging.is_exhausted() || every_lane_waits || !open;
			if !self.pending.is_empty() && due {
				self.dispatch();
			}
		}
	}

	/// Queues a staged upload for the next batch, or starts a native read.
	fn accept(&mut self, request: UploadRequest) {
		match request {
			UploadRequest::Staged(upload) => self.pending.push(upload),
			UploadRequest::NativeImage { upload, reply } => {
				if let Err(error) = self.submit_native(upload, reply.clone()) {
					let _ = reply.send(Err(error));
				}
			}
		}
	}

	/// Creates every image of the pending batch, records all of its copies in one workload, and answers every lane
	/// once it landed.
	///
	/// A lane that stopped waiting drops its images when its reply fails to send.
	fn dispatch(&mut self) {
		let mut pending = std::mem::take(&mut self.pending);
		let mut answers = SmallVec::<[_; 16]>::new();
		for StagedUpload {
			staging,
			images,
			buffers,
			reply,
		} in pending.drain(..)
		{
			let base = staging.offset();
			let mut handles = SmallVec::<[ghi::ImageHandle; 2]>::new();
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
			answers.push((handles, staging, reply));
		}
		self.pending = pending;

		let (buffer_copies, image_copies) = (&self.buffer_copies, &self.image_copies);
		self.context.record(self.queue, Some("Loader Batch"), |recording| {
			recording.copy_buffers(buffer_copies);
			recording.copy_buffer_to_images(image_copies);
		});
		self.image_copies.clear();
		self.buffer_copies.clear();

		// The copies landed, so the staging goes back to the arena and the images can leave the context.
		for (images, staging, reply) in answers {
			drop(staging);
			let images = images.into_iter().map(|image| self.context.export_image(image)).collect();
			let _ = reply.send(images);
		}
	}

	/// Answers the lanes whose native reads finished, with their images or with why a read failed.
	fn finish_reads(&mut self) {
		let mut index = 0;
		while index < self.reads.len() {
			if self.reads[index].ticket.status() == ghi::io::ResourceIoStatus::Pending {
				index += 1;
				continue;
			}
			let NativeRead {
				image,
				file,
				ticket,
				name,
				reply,
			} = self.reads.swap_remove(index);
			let image = self.context.export_image(image);
			let result = ticket.wait();
			if let Some(queue) = &mut self.io_queue {
				queue.close_file(file);
			}
			let result = match result {
				Ok(()) => {
					log::debug!("Streamed texture '{name}' through native I/O.");
					Ok(image)
				}
				Err(error) => Err(format!(
					"Texture I/O failed for {name}. The most likely cause is unreadable or incompatible compressed texture data. {error}"
				)),
			};
			let _ = reply.send(result);
		}
	}

	/// Creates the image for a native upload, opens its file, and submits one read per mip on the native I/O queue,
	/// which [`Self::finish_reads`] answers on `reply` once they finish.
	fn submit_native(
		&mut self,
		upload: NativeImageUpload,
		reply: kanal::Sender<Result<ghi::implementation::DetachedImage, String>>,
	) -> Result<(), String> {
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
		let format = upload.description.format;
		let requests = upload
			.regions
			.iter()
			.map(|region| {
				// Each backend reads its own source layout: compact rows on Metal, the D3D12 copy footprint on DX12.
				let layout = self.context.resource_io_image_source_layout(format, region.extent)?;
				let mut source = ghi::io::ResourceIoFileRegion::new(file, region.file_offset);
				if let Some((offset, size)) = region.stored {
					source = source.stored_range(offset, size);
				}
				Ok(ghi::io::ResourceIoImageLoad::new(
					source,
					image,
					0,
					region.mip_level,
					region.extent,
					layout.bytes_per_row,
					layout.bytes_per_image,
				)
				.origin(Extent::new(0, region.first_row, 0))
				.into())
			})
			.collect::<Result<SmallVec<[ghi::io::ResourceIoRequest; 16]>, ghi::io::ResourceIoError>>();
		let submission = requests.and_then(|requests| queue.submit(&self.context, Some(name), &requests));
		match submission {
			Ok(ticket) => {
				self.reads.push(NativeRead {
					image,
					file,
					ticket,
					name: upload.description.name,
					reply,
				});
				Ok(())
			}
			Err(error) => {
				// Nothing will fill the image or read the file, so free both.
				queue.close_file(file);
				drop(self.context.export_image(image));
				Err(format!(
					"Texture I/O submission failed for {name}. The most likely cause is an unsupported request or unavailable native queue. {error}"
				))
			}
		}
	}
}
