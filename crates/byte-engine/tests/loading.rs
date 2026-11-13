//! Exercises the loader on the active native GHI backend.
//!
//! These tests need a GPU, so the default nextest filter skips this binary. Run them with
//! `cargo nextest run -p byte-engine --test loading --ignore-default-filter`.

#![cfg(feature = "headed")]

use std::{
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
	},
	time::{Duration, Instant},
};

use byte_engine::{
	application::{
		Parameter,
		graphics::defaults::{build_single_threaded_async_runtime, drive_runtime},
		parameters::Parameters,
	},
	rendering::{
		GraphicsDevice,
		loading::{BufferRegion, Event, LoadError, LoadPipeline, Loader, LoaderClient, LoaderLane, spawn},
		resource_loading::load_texture,
	},
};
use ghi::{
	command_buffer::CommandBufferRecording as _,
	context::{Context as _, ContextCreate as _},
	queue::Queue as _,
};
use resource_management::{
	ProcessedAsset, StreamDescription,
	asset::ResourceId,
	resource::{MemoryStorageBackend, WriteStorageBackend as _, resource_manager::ResourceManager},
	resources::image::Image,
	types::{Formats, Gamma},
};
use utils::Extent;

/// Each upload fills this many bytes, so a few of them exceed the loader's staging arena.
const CHUNK: usize = 8 * 1024 * 1024;
const UPLOADS: usize = 6;

/// The `NoParameters` struct gives the loader and device their defaults.
struct NoParameters;

impl Parameters for NoParameters {
	fn get_parameter(&self, _name: &str) -> Option<&Parameter> {
		None
	}
}

/// The `ChunkPipeline` struct fills chunk `n` of one shared render buffer with the byte `n + 1`.
struct ChunkPipeline {
	destination: ghi::BaseBufferHandle,
}

impl LoadPipeline for ChunkPipeline {
	type Key = usize;
	type Request = usize;
	type Resident = usize;

	fn key(request: &usize) -> usize {
		*request
	}

	async fn load(&self, chunk: usize, lane: &mut LoaderLane<Self>) -> Result<usize, LoadError> {
		let mut staging = lane
			.staging()
			.allocate(CHUNK, 256)
			.await
			.ok_or_else(|| LoadError("The test chunk does not fit the loader's staging arena.".to_string()))?;
		staging.bytes_mut().fill(chunk as u8 + 1);
		let region = BufferRegion {
			offset: 0,
			destination: self.destination,
			destination_offset: chunk * CHUNK,
			size: CHUNK,
		};
		lane.upload(staging, [], smallvec::smallvec![region]).await?;
		Ok(chunk)
	}
}

/// The `TreePipeline` struct loads node `n` of a small tree and requests its children, 2n + 1 and 2n + 2.
///
/// Nodes 1 and 2 both request node 3 as well, so it has two parents.
struct TreePipeline {
	loads: Arc<Mutex<Vec<u32>>>,
}

impl LoadPipeline for TreePipeline {
	type Key = u32;
	type Request = u32;
	type Resident = u32;

	fn key(request: &u32) -> u32 {
		*request
	}

	async fn load(&self, node: u32, lane: &mut LoaderLane<Self>) -> Result<u32, LoadError> {
		self.loads.lock().unwrap().push(node);
		for child in [2 * node + 1, 2 * node + 2] {
			if child < 7 {
				lane.request(child);
			}
		}
		if node == 1 || node == 2 {
			lane.request(3);
		}
		Ok(node)
	}
}

/// The side of [`TEXTURE_ID`]'s base level. Its mips are 32, 16, 8, 4, 2, and 1 texels wide.
const TEXTURE_SIDE: u32 = 32;
const TEXTURE_MIPS: u32 = 6;
const TEXTURE_ID: &str = "texture.image";

/// The `TexturePipeline` struct loads [`TEXTURE_ID`] from the mip level each request names.
struct TexturePipeline {
	resources: ResourceManager,
}

impl LoadPipeline for TexturePipeline {
	type Key = u32;
	type Request = u32;
	type Resident = ghi::implementation::DetachedImage;

	fn key(request: &u32) -> u32 {
		*request
	}

	async fn load(&self, first_level: u32, lane: &mut LoaderLane<Self>) -> Result<Self::Resident, LoadError> {
		let reference = self
			.resources
			.request::<Image>(TEXTURE_ID)
			.await
			.map_err(|error| LoadError(format!("The test texture is not stored. {error}")))?;
		load_texture(reference, TEXTURE_ID, first_level, lane)
			.await
			.map_err(|error| LoadError(error.to_string()))
	}
}

/// Stores an RGBA8 image whose every byte in mip `n` is `n + 1`, so a read texel names the mip it came from.
async fn store_level_numbered_texture(storage: &MemoryStorageBackend) {
	let mut streams = Vec::new();
	let mut bytes = Vec::new();
	for level in 0..TEXTURE_MIPS {
		let side = (TEXTURE_SIDE >> level) as usize;
		streams.push(StreamDescription::new(format!("mip[{level}]"), side * side * 4, bytes.len()));
		bytes.resize(bytes.len() + side * side * 4, level as u8 + 1);
	}
	let image = Image {
		format: Formats::RGBA8,
		gamma: Gamma::Linear,
		extent: [TEXTURE_SIDE, TEXTURE_SIDE, 0],
		mip_count: TEXTURE_MIPS,
		cpu_readable: false,
		ibl: None,
		photometry: None,
	};
	storage
		.store(
			ProcessedAsset::new(ResourceId::new(TEXTURE_ID), image).with_streams(streams),
			&bytes,
		)
		.await
		.expect("The test texture should store.");
}

/// Runs the loader and its lanes on a thread of their own, the way the loading thread does, until `stop` is set.
fn run_loading_thread<P: LoadPipeline>(
	loader: Loader,
	lanes: Vec<LoaderLane<P>>,
	stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
	std::thread::spawn(move || {
		let runtime = build_single_threaded_async_runtime();
		runtime.enter(|| {
			loader.run(&runtime);
			for lane in lanes {
				runtime.spawn(lane.run()).detach();
			}
			drive_runtime(&runtime, || stop.load(Ordering::Relaxed));
		});
	})
}

/// Polls `client` until `count` resources are ready, failing the test on an error or when loading stalls.
fn poll_ready<P: LoadPipeline>(client: &mut LoaderClient<P>, count: usize, mut adopt: impl FnMut(P::Resident)) -> usize {
	let deadline = Instant::now() + Duration::from_secs(60);
	let mut ready = 0;
	while ready < count {
		assert!(
			Instant::now() < deadline,
			"Loading stalled after {ready} of {count} resources. The most likely cause is that a batch was never dispatched."
		);
		match client.poll() {
			Some(Event::Ready { resident, .. }) => {
				adopt(resident);
				ready += 1;
			}
			Some(Event::Failed { error, .. }) => panic!("{error}"),
			None => std::thread::sleep(Duration::from_millis(1)),
		}
	}
	ready
}

#[test]
fn uploads_are_visible_when_ready_even_beyond_staging_capacity() {
	//! Tests that every upload finishes when all of them together need more staging than the loader has, and that a
	//! resource is only reported ready once its bytes are in the render buffer.

	let device = GraphicsDevice::new(&NoParameters);
	let mut render = device.create_context();
	let destination: ghi::BufferHandle<[u8]> = render.build_buffer(
		ghi::buffer::Builder::new(ghi::Uses::TransferDestination)
			.name("Loaded Chunks")
			.length(CHUNK * UPLOADS)
			.device_accesses(ghi::DeviceAccesses::HostOnly),
	);
	let mut loader = Loader::new(&device, &NoParameters);
	let imported = loader.import_buffer(render.share_buffer(destination)).into();
	let (mut client, lanes) = spawn(&loader, ChunkPipeline { destination: imported }, 3, 1, UPLOADS);

	let stop = Arc::new(AtomicBool::new(false));
	let thread = run_loading_thread(loader, lanes, stop.clone());

	for chunk in 0..UPLOADS {
		client.request(chunk);
	}
	poll_ready(&mut client, UPLOADS, |chunk| {
		let bytes = &render.get_buffer_slice(destination)[chunk * CHUNK..][..CHUNK];
		assert!(
			bytes.iter().all(|byte| *byte == chunk as u8 + 1),
			"Chunk {chunk} was reported ready before its copy finished."
		);
	});

	stop.store(true, Ordering::Relaxed);
	thread.join().unwrap();
	assert!(!render.has_errors());
}

#[test]
fn dependencies_load_once_without_render_requests() {
	//! Tests that lanes schedule the dependencies a load names, and that a resource with two parents loads once.
	//! Only the root is requested from the render side, so every other result comes from lane requests.

	const NODES: usize = 7;

	let device = GraphicsDevice::new(&NoParameters);
	let loader = Loader::new(&device, &NoParameters);
	let loads = Arc::new(Mutex::new(Vec::new()));
	let (mut client, lanes) = spawn(&loader, TreePipeline { loads: loads.clone() }, 2, 1, NODES);

	let stop = Arc::new(AtomicBool::new(false));
	let thread = run_loading_thread(loader, lanes, stop.clone());

	client.request(0);
	let mut ready = Vec::new();
	poll_ready(&mut client, NODES, |node| ready.push(node));
	ready.sort_unstable();
	assert_eq!(ready, (0..NODES as u32).collect::<Vec<_>>());

	stop.store(true, Ordering::Relaxed);
	thread.join().unwrap();
	let mut loads = loads.lock().unwrap().clone();
	loads.sort_unstable();
	assert_eq!(
		loads,
		(0..NODES as u32).collect::<Vec<_>>(),
		"Every node must load exactly once."
	);
}

#[test]
fn textures_load_from_a_first_mip_level() {
	//! Tests that a texture loaded from a later mip is an image whose base level holds that mip, with that mip's
	//! extent, so it samples like the resource with only its finer mips missing.

	let storage = MemoryStorageBackend::new();
	build_single_threaded_async_runtime().block_on(store_level_numbered_texture(&storage));
	let device = GraphicsDevice::new(&NoParameters);
	let mut render = device.create_context();
	let loader = Loader::new(&device, &NoParameters);
	let first_levels = [0, 2, TEXTURE_MIPS - 1];
	let (mut client, lanes) = spawn(
		&loader,
		TexturePipeline {
			resources: ResourceManager::new(storage),
		},
		2,
		1,
		first_levels.len(),
	);

	let stop = Arc::new(AtomicBool::new(false));
	let thread = run_loading_thread(loader, lanes, stop.clone());

	for first_level in first_levels {
		client.request(first_level);
	}
	let mut images = Vec::new();
	let deadline = Instant::now() + Duration::from_secs(60);
	while images.len() < first_levels.len() {
		assert!(Instant::now() < deadline, "Loading stalled after {} textures.", images.len());
		match client.poll() {
			Some(Event::Ready { key, resident }) => images.push((key, render.intern_image(resident))),
			Some(Event::Failed { error, .. }) => panic!("{error}"),
			None => std::thread::sleep(Duration::from_millis(1)),
		}
	}
	stop.store(true, Ordering::Relaxed);
	thread.join().unwrap();

	let command_buffer = render.queue(device.graphics_queue()).create_command_buffer(None);
	let finished = render.create_synchronizer(None, false);
	let mut recording = render.create_command_buffer_recording(command_buffer);
	let transfers = images
		.iter()
		.map(|&(first_level, image)| (first_level, recording.transfer_texture(image.into()).unwrap()))
		.collect::<Vec<_>>();
	recording.execute(finished);
	render.wait_for_synchronizer(finished);

	for (first_level, transfer) in transfers {
		let readback = render.get_image_data(transfer).unwrap();
		let side = TEXTURE_SIDE >> first_level;
		assert_eq!(readback.extent, Extent::square(side), "first level {first_level}");
		for row in 0..side as usize {
			let texels = &readback.bytes[row * readback.bytes_per_row..][..side as usize * 4];
			assert!(
				texels.iter().all(|byte| *byte == first_level as u8 + 1),
				"The base level of the image loaded from mip {first_level} holds another mip's texels."
			);
		}
	}
	assert!(!render.has_errors());
}

#[test]
fn reloads_load_a_resident_resource_again() {
	//! Tests that a reload runs the load again for a resource that is already resident, and that the resources it
	//! depends on stay resident instead of loading again.

	let device = GraphicsDevice::new(&NoParameters);
	let loader = Loader::new(&device, &NoParameters);
	let loads = Arc::new(Mutex::new(Vec::new()));
	let (mut client, lanes) = spawn(&loader, TreePipeline { loads: loads.clone() }, 2, 1, 8);

	let stop = Arc::new(AtomicBool::new(false));
	let thread = run_loading_thread(loader, lanes, stop.clone());

	// Node 2 depends on nodes 5 and 6, and on node 3.
	client.request(2);
	poll_ready(&mut client, 4, |_| {});
	client.reload(2);
	let mut reloaded = Vec::new();
	poll_ready(&mut client, 1, |node| reloaded.push(node));

	stop.store(true, Ordering::Relaxed);
	thread.join().unwrap();
	assert_eq!(reloaded, [2]);
	let mut loads = loads.lock().unwrap().clone();
	loads.sort_unstable();
	assert_eq!(loads, [2, 2, 3, 5, 6], "Only the reloaded node must load twice.");
}
