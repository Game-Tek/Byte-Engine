use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
	CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
};
use ghi::context::ContextCreate as _;
use ghi::frame::Frame as _;
use resource_management::resources::particle_system::ParticleSystem;
use smallvec::SmallVec;
use utils::Extent;
use utils::hash::HashMap;

use super::emitter::ParticleEmitter;
use super::loader::ParticleSystemLoader;
use super::shader_data::{
	DISPATCH_SLOT, DRAWS_SLOT, FRAME_SLOT, MAX_EMITTERS, PARTICLES_SLOT, ParticleFrameData, ParticlePushConstants,
	ShaderParticle, ShaderSpawn,
};
use crate::core::factory::{CreateMessage, Handle};
use crate::core::listener::{DefaultListener, Listener as _};
use crate::core::message::DeleteMessage;
use crate::gameplay::transform::TransformationUpdate;
use crate::gameplay::world::DefaultWorld;
use crate::rendering::loading::{Event, LoaderClient};
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::render_pass::{RenderPassBuilder, RenderPassReturn, allocate_render_command};
use crate::rendering::{PipelineManagerClient, PipelineRef, Sink};
use crate::time::MediaTime;

/// The longest step one frame simulates, in seconds. A longer hitch slows particles down instead of scattering them.
const LONGEST_STEP: f32 = 0.1;

/// The `ParticleManager` struct runs every particle system the world's emitters use: it loads each `.particles`
/// system once, turns emitters into spawn requests, and records the GPU simulation and draw for every camera.
///
/// Register it after the scene pipeline whose `main` color and `depth` it draws into, through
/// [`crate::application::graphics::setup_particles`]. Emitters arrive as [`ParticleEmitter`] creations on the world
/// it was built from.
pub struct ParticleManager {
	systems_loader: LoaderClient<ParticleSystemLoader>,
	emitter_listener: DefaultListener<CreateMessage<ParticleEmitter>>,
	transforms_listener: DefaultListener<TransformationUpdate>,
	deletions_listener: DefaultListener<DeleteMessage>,
	emitters: HashMap<Handle, EmitterState>,
	/// Every system an emitter asked for, in request order. Emitters refer to their system by index.
	systems: Vec<SystemEntry>,
	pipeline_manager: PipelineManagerClient,
	prepare_pipeline: PipelineRef,
	/// Each sink's color and depth attachments, by sink id.
	sink_attachments: SmallVec<[(usize, [ghi::AttachmentInformation; 2]); 4]>,
	/// Simulated seconds. Retirement and idleness are measured on this clock, which hitches slow down like particles.
	simulated_time: f64,
	last_frame_time: Option<MediaTime>,
	seed: u32,
}

/// The `EmitterState` struct is one live emitter's CPU state.
struct EmitterState {
	emitter: ParticleEmitter,
	system: usize,
	/// The emitter's slot in its system's pool, once the system has loaded and a slot was free.
	slot: Option<usize>,
	model: ghi::pod::Mat4x3f,
	/// Spawn debt carried between frames, so low rates still emit at the right average.
	fractional_spawn: f32,
	/// Publishes whose burst is not spawned yet.
	pending_bursts: u32,
	/// Whether the emitter already reported that its system had no free slot.
	reported_full: bool,
}

/// The `SystemEntry` struct is one requested system, with its GPU pool once the system has loaded.
struct SystemEntry {
	id: String,
	pool: Option<Box<SystemPool>>,
}

/// The `SystemPool` struct owns one loaded system's GPU state and emitter slots.
struct SystemPool {
	system: ParticleSystem,
	simulate_pipeline: PipelineRef,
	draw_pipeline: PipelineRef,
	descriptor_set: ghi::DescriptorSetHandle,
	frame_data: ghi::DynamicBufferHandle<ParticleFrameData>,
	draws: ghi::BufferHandle<[[u32; 4]; 2]>,
	dispatch: ghi::BufferHandle<[[u32; 3]; 1]>,
	/// A particle stores its emitter's slot, so a slot stays taken until its last particle dies.
	slots: [Slot; MAX_EMITTERS],
	/// The buffer half the last simulation wrote.
	side: u32,
	/// Whether the buffers hold no particles worth keeping, so the next simulation starts empty.
	reset: bool,
	/// The simulated time after which no particle can still be alive.
	alive_until: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum Slot {
	Free,
	Emitting,
	/// Released by its emitter; free again at the given simulated time, when its last particle has died.
	Retiring(f64),
}

/// The `PoolFrame` struct is what one system's commands need in a frame.
#[derive(Clone, Copy)]
struct PoolFrame {
	/// The index of the system in [`ParticleManager::systems`].
	system: usize,
	simulate: ghi::PipelineHandle,
	draw: ghi::PipelineHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	draws: ghi::BufferHandle<[[u32; 4]; 2]>,
	dispatch: ghi::BufferHandle<[[u32; 3]; 1]>,
	side: u32,
}

impl ParticleManager {
	/// Requests the shared prepare pipeline and subscribes to the world's emitters. Next, register the manager with
	/// [`crate::rendering::Renderer::add_pipeline_manager`].
	pub(crate) fn new(
		world: &DefaultWorld,
		pipeline_manager: PipelineManagerClient,
		systems_loader: LoaderClient<ParticleSystemLoader>,
	) -> Self {
		Self {
			systems_loader,
			emitter_listener: world.factory::<ParticleEmitter>().listener(),
			transforms_listener: world.transforms_channel().listener(),
			deletions_listener: world.deletions_listener(),
			emitters: HashMap::default(),
			systems: Vec::new(),
			prepare_pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/particles/prepare.pipeline"),
			pipeline_manager,
			sink_attachments: SmallVec::new(),
			simulated_time: 0.0,
			last_frame_time: None,
			seed: 0,
		}
	}

	/// Applies emitter creations, replacements, transforms, and deletions published since the last call.
	fn adopt_messages(&mut self) {
		while let Some(message) = self.emitter_listener.read() {
			let handle = message.handle();
			let emitter = message.into_data();
			let system = self.system_index(&emitter.system);
			match self.emitters.get_mut(&handle) {
				// A replacement keeps the slot, so particles already in flight keep their emitter.
				Some(state) if state.system == system => {
					state.emitter = emitter;
					state.pending_bursts += 1;
				}
				// A new emitter, or one that switched systems and gives its old slot back.
				_ => {
					let mut model = math::Matrix::identity().into();
					if let Some(previous) = self.emitters.remove(&handle) {
						model = previous.model;
						release(&mut self.systems, previous, self.simulated_time);
					}
					self.emitters.insert(
						handle,
						EmitterState {
							emitter,
							system,
							slot: None,
							model,
							fractional_spawn: 0.0,
							pending_bursts: 1,
							reported_full: false,
						},
					);
				}
			}
		}
		while let Some(message) = self.transforms_listener.read() {
			if let Some(state) = self.emitters.get_mut(&message.handle()) {
				state.model = message.transform().get_matrix().into();
			}
		}
		while let Some(message) = self.deletions_listener.read() {
			if let Some(state) = self.emitters.remove(&message.into_handle()) {
				release(&mut self.systems, state, self.simulated_time);
			}
		}
	}

	/// Returns the index of the system `id`, requesting it from the loader the first time an emitter names it.
	fn system_index(&mut self, id: &str) -> usize {
		if let Some(index) = self.systems.iter().position(|entry| entry.id == id) {
			return index;
		}
		self.systems_loader.request(id.to_string());
		self.systems.push(SystemEntry {
			id: id.to_string(),
			pool: None,
		});
		self.systems.len() - 1
	}

	/// Creates the GPU pools of systems that finished loading.
	fn adopt_loaded_systems(&mut self, frame: &mut ghi::implementation::Frame) {
		while let Some(event) = self.systems_loader.poll() {
			match event {
				Event::Ready { key, resident } => {
					if let Some(entry) = self.systems.iter_mut().find(|entry| entry.id == key) {
						entry.pool = Some(Box::new(SystemPool::new(frame, &self.pipeline_manager, resident)));
					}
				}
				Event::Failed { key, error } => {
					log::warn!(
						"Particle system '{key}' is not drawn: {error} The most likely cause is a missing or invalid `.particles` asset."
					);
				}
			}
		}
	}

	/// Frees slots whose last particle has died, then gives slots to emitters whose system is ready.
	fn assign_slots(&mut self) {
		for pool in self.systems.iter_mut().filter_map(|entry| entry.pool.as_deref_mut()) {
			for slot in &mut pool.slots {
				if matches!(*slot, Slot::Retiring(free_at) if free_at <= self.simulated_time) {
					*slot = Slot::Free;
				}
			}
		}
		for state in self.emitters.values_mut().filter(|state| state.slot.is_none()) {
			let entry = &mut self.systems[state.system];
			let Some(pool) = entry.pool.as_deref_mut() else {
				continue;
			};
			state.slot = pool.slots.iter().position(|slot| *slot == Slot::Free);
			if let Some(slot) = state.slot {
				pool.slots[slot] = Slot::Emitting;
			}
			crate::rendering::warn_once(&mut state.reported_full, state.slot.is_none(), || {
				format!(
					"Particle emitter waits for a slot: all {MAX_EMITTERS} emitter slots of '{}' are taken. The most likely cause is that too many of its emitters are alive, or were deleted too recently for their particles to have died.",
					entry.id
				)
			});
		}
	}
}

impl SystemPool {
	/// Creates the buffers of one loaded system and requests its generated pipelines.
	fn new(frame: &mut ghi::implementation::Frame, pipeline_manager: &PipelineManagerClient, system: ParticleSystem) -> Self {
		let frame_data = frame.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Particle Frame")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// Two halves: the simulation reads last frame's particles from one and packs the survivors into the other.
		let particles = frame.build_buffer::<[ShaderParticle]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Particles")
				.length(system.capacity as usize * 2)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let draws = frame.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::Indirect)
				.name("Particle Draws")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let dispatch = frame.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::Indirect)
				.name("Particle Simulation Dispatch")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let descriptor_set = frame.create_descriptor_set(Some("Particle Descriptor Set"));
		frame.write(&[
			ghi::DescriptorWrite::buffer(descriptor_set, FRAME_SLOT, frame_data.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, PARTICLES_SLOT, particles.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, DRAWS_SLOT, draws.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, DISPATCH_SLOT, dispatch.into()),
		]);

		Self {
			simulate_pipeline: pipeline_manager.request_pipeline(&system.simulate_pipeline),
			draw_pipeline: pipeline_manager.request_pipeline(&system.draw_pipeline),
			system,
			descriptor_set,
			frame_data,
			draws,
			dispatch,
			slots: [Slot::Free; MAX_EMITTERS],
			side: 0,
			reset: true,
			alive_until: f64::NEG_INFINITY,
		}
	}
}

/// Gives back a removed emitter's slot. Its particles keep the slot until the last of them has died.
fn release(systems: &mut [SystemEntry], state: EmitterState, simulated_time: f64) {
	if let (Some(slot), Some(pool)) = (state.slot, systems[state.system].pool.as_deref_mut()) {
		pool.slots[slot] = Slot::Retiring(simulated_time + f64::from(pool.system.longest_life));
	}
}

/// Writes one system's spawn runs and spawning emitters into `data` for a simulation that fills `side`, and returns
/// how many particles were requested.
fn write_frame_data(
	pool: &mut SystemPool,
	system: usize,
	emitters: &mut HashMap<Handle, EmitterState>,
	data: &mut ParticleFrameData,
	side: u32,
	delta_time: f32,
	seed: u32,
) -> u32 {
	let mut spawn_total = 0u32;
	let mut spawn_count = 0usize;
	for state in emitters.values_mut().filter(|state| state.system == system) {
		let Some(slot) = state.slot else {
			continue;
		};
		let rate = state.emitter.rate.unwrap_or(pool.system.rate).max(0.0);
		let spawn = state.fractional_spawn + rate * delta_time;
		let streamed = spawn.floor();
		state.fractional_spawn = spawn - streamed;
		let burst = state.emitter.burst.unwrap_or(pool.system.burst);
		let count = (streamed as u32).saturating_add(burst.saturating_mul(std::mem::take(&mut state.pending_bursts)));
		if count == 0 {
			continue;
		}
		// Only spawning emitters need their transform: live particles never read it again.
		data.emitters[slot] = state.model;
		data.spawns[spawn_count] = ShaderSpawn {
			first: spawn_total,
			emitter: slot as u32,
		};
		spawn_count += 1;
		spawn_total = spawn_total.saturating_add(count);
	}

	data.delta_time = delta_time;
	data.seed = seed;
	data.spawn_total = spawn_total;
	data.spawn_count = spawn_count as u32;
	data.side = side;
	data.reset = u32::from(pool.reset);
	data.capacity = pool.system.capacity;
	spawn_total
}

impl PipelineManager for ParticleManager {
	/// Drains world messages every tick, so a window that draws no frames never holds back the world's channels.
	fn update(&mut self) {
		self.adopt_messages();
	}

	/// Drains world messages after each simulation step, which may publish many transforms per tick.
	fn step(&mut self) {
		self.adopt_messages();
	}

	/// Advances the simulation clock, then records every system's simulation once and its draw for every sink.
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
		_alpha: f32,
		time: MediaTime,
	) -> SmallVec<[(usize, RenderPassReturn<'a>); 16]> {
		let mut commands = SmallVec::new();
		self.adopt_messages();
		self.adopt_loaded_systems(frame);

		let delta_time = self.last_frame_time.map_or(0.0, |last| {
			(time.as_seconds_f32() - last.as_seconds_f32()).clamp(0.0, LONGEST_STEP)
		});
		self.last_frame_time = Some(time);
		self.simulated_time += f64::from(delta_time);
		self.assign_slots();

		let Some(prepare) = self.pipeline_manager.pipeline(self.prepare_pipeline) else {
			return commands;
		};
		if sinks.is_empty() {
			return commands;
		}

		self.seed = self.seed.wrapping_add(1);
		let mut pools = bumpalo::collections::Vec::with_capacity_in(self.systems.len(), frame_allocator);
		for (index, entry) in self.systems.iter_mut().enumerate() {
			let Some(pool) = entry.pool.as_deref_mut() else {
				continue;
			};
			let (Some(simulate), Some(draw)) = (
				self.pipeline_manager.pipeline(pool.simulate_pipeline),
				self.pipeline_manager.pipeline(pool.draw_pipeline),
			) else {
				continue;
			};
			// The simulation reads the half the last one filled and packs survivors into the other.
			let side = pool.side ^ 1;
			let frame_data = pool.frame_data;
			let spawn_total = write_frame_data(
				pool,
				index,
				&mut self.emitters,
				frame.get_mut_dynamic_buffer_slice(frame_data),
				side,
				delta_time,
				self.seed,
			);
			// New particles keep the system busy until the longest-lived of them dies. With nothing alive and nothing
			// new, the system has no GPU work. Its buffers then hold stale particles, so its next simulation must start
			// from empty.
			if spawn_total > 0 {
				pool.alive_until = pool
					.alive_until
					.max(self.simulated_time + f64::from(pool.system.longest_life));
			} else if self.simulated_time > pool.alive_until {
				pool.reset = true;
				continue;
			}
			frame.sync_buffer(frame_data);
			pools.push(PoolFrame {
				system: index,
				simulate,
				draw,
				descriptor_set: pool.descriptor_set,
				draws: pool.draws,
				dispatch: pool.dispatch,
				side,
			});
		}
		if pools.is_empty() {
			return commands;
		}
		let pools: &'a [PoolFrame] = pools.into_bump_slice();

		for sink in sinks {
			let Some(&(_, attachments)) = self.sink_attachments.iter().find(|(sink_id, _)| *sink_id == sink.index()) else {
				continue;
			};
			// The translation column of the inverse view is the camera's world position.
			let camera = math::inverse(sink.view().view()).get_column(3);
			let push_constants = ParticlePushConstants {
				view_projection: sink.view_projection().into(),
				camera: [camera.x, camera.y, camera.z, sink.exposure_scale()],
			};
			// Every sink sees the same particles, so only the first recorded one simulates them.
			let simulates = commands.is_empty();
			let extent = sink.extent();

			let command = allocate_render_command(frame_allocator, move |c| {
				c.region(
					|label| label.write_str("Particles"),
					|c| {
						if simulates {
							// Every prepare pass first, so the simulations that read their results wait on one barrier.
							for pool in pools {
								let c = c.bind_compute_pipeline(prepare);
								c.bind_descriptor_sets(&[pool.descriptor_set]);
								c.dispatch(ghi::DispatchExtent::new(Extent::line(1), Extent::line(1)));
							}
							for pool in pools {
								let c = c.bind_compute_pipeline(pool.simulate);
								c.bind_descriptor_sets(&[pool.descriptor_set]);
								c.indirect_dispatch(pool.dispatch, 0);
							}
						}

						let render_pass = c.start_render_pass(extent, &attachments);
						for pool in pools {
							let c = render_pass.bind_raster_pipeline(pool.draw);
							c.bind_descriptor_sets(&[pool.descriptor_set]);
							c.write_push_constant(0, push_constants);
							c.draw_indirect(pool.draws, pool.side as usize);
						}
						render_pass.end_render_pass();
					},
				);
			});
			commands.push((sink.index(), command));
		}

		// A frame that recorded nothing leaves the live particles in the half they were in.
		if !commands.is_empty() {
			for pool_frame in pools {
				if let Some(pool) = self.systems[pool_frame.system].pool.as_deref_mut() {
					pool.side = pool_frame.side;
					pool.reset = false;
				}
			}
		}
		commands
	}

	/// Records which images particles draw into for a new sink: the scene's `main` color and `depth`.
	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder) {
		let color = render_pass_builder.render_to("main");
		let depth = render_pass_builder.read_from("depth");
		// Particles test against the scene's depth without writing it, so they hide behind geometry and never hide
		// each other.
		let attachments = [color.into(), depth.into()].map(|image: ghi::ImageOrSwapchain| {
			ghi::AttachmentInformation::new(image, ghi::Layouts::RenderTarget, ghi::LoadOp::Load, ghi::StoreOp::Store)
		});
		self.sink_attachments.push((sink_id, attachments));
	}
}
