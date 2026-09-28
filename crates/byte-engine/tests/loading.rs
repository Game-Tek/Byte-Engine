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
	},
};
use ghi::context::{Context as _, ContextCreate as _};

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
	let (mut client, lanes) = spawn(&loader, ChunkPipeline { destination: imported }, 3, UPLOADS);

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
	let (mut client, lanes) = spawn(&loader, TreePipeline { loads: loads.clone() }, 2, NODES);

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
fn reloads_load_a_resident_resource_again() {
	//! Tests that a reload runs the load again for a resource that is already resident, and that the resources it
	//! depends on stay resident instead of loading again.

	let device = GraphicsDevice::new(&NoParameters);
	let loader = Loader::new(&device, &NoParameters);
	let loads = Arc::new(Mutex::new(Vec::new()));
	let (mut client, lanes) = spawn(&loader, TreePipeline { loads: loads.clone() }, 2, 8);

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
