//! Runs an authored animation graph in a headed application.
//!
//! Replace the three resource IDs below with a skinned mesh and two compatible
//! animation clips from your resource store. The mesh and clips must use unique
//! node names for [`SkeletonPoseMap`](resource_management::resources::skeleton::SkeletonPoseMap)
//! to map their poses.

use std::num::NonZeroUsize;

use byte_engine::{
	animation::graph::{
		AnimationClip, AnimationGraph, AnimationGraphPlayer, AnimationPool, AnimationPoolConfig, AnimationTransition,
	},
	application::{
		graphics::{
			setup_default_audio, setup_default_input, setup_default_resource_and_asset_management,
			setup_pbr_visibility_shading_render_pipeline, GraphicsApplication,
		},
		Application, Parameter,
	},
	audio::AudioSamplePoolConfig,
	core::{channel::Channel as _, EntityHandle},
	gameplay::{Object, Transform, TransformationUpdate},
	rendering::{window::Window, Camera, UpdatePose},
	MediaTime,
};
use math::{Point, Vector};
use resource_management::resources::mesh::Mesh;
use utils::Extent;

// These assets are intentionally explicit. Replace them before running the example.
const MODEL_RESOURCE: &str = "replace/with/skinned-model.fbx";
const IDLE_ANIMATION: &str = "replace/with-idle-animation.fbx";
const WALK_ANIMATION: &str = "replace/with-walk-animation.fbx";
const ROOT_MOTION_NODE: Option<usize> = Some(0);
const ANIMATION_POOL_BYTES: usize = 32 * 1024 * 1024;

/// The `LocomotionInput` struct holds the game-owned facts used by graph predicates.
#[derive(Clone, Copy)]
struct LocomotionInput {
	moving: bool,
}

/// Builds the reusable graph once during application setup.
fn locomotion_graph() -> AnimationGraph<LocomotionInput> {
	let mut builder = AnimationGraph::builder();
	let idle = builder.state("idle", AnimationClip::looping(IDLE_ANIMATION));
	let walk = builder.state("walk", AnimationClip::looping(WALK_ANIMATION));

	// The player checks transitions in authoring order. Each transition keeps the
	// current clip visible while its target clip loads in the shared pool.
	builder
		.transition(
			idle,
			walk,
			AnimationTransition::when(|input: &LocomotionInput| input.moving).inertialize(MediaTime::from_millis(150)),
		)
		.transition(
			walk,
			idle,
			AnimationTransition::when(|input: &LocomotionInput| !input.moving).inertialize(MediaTime::from_millis(150)),
		);

	builder
		.build(idle)
		.expect("the example graph has valid state IDs and clip IDs")
}

fn main() {
	let mut app = GraphicsApplication::new(
		"Animation Graph",
		&[
			Parameter::new("render.ghi.features.mesh-shading", "false"),
			Parameter::new("resources.path", "resources"),
		],
	);

	// Keep the deferred-task queue until every resource worker is registered.
	// `default_setup` launches this queue internally, so this explicit setup form
	// is the one to use when an app also owns animation-loading workers.
	let mut loading_tasks = byte_engine::application::graphics::defaults::build_deferred_tasks_queue();
	#[cfg(debug_assertions)]
	{
		use byte_engine::rendering::pipelines::visibility::shader_generator::VisibilityShaderGenerator;

		setup_default_resource_and_asset_management(
			&mut app,
			VisibilityShaderGenerator::new(true, false, false, false, false, false, true, true),
		);
	}
	setup_default_input(&mut app);
	setup_default_audio(&mut app, AudioSamplePoolConfig::default(), |task| loading_tasks.push(task));
	setup_pbr_visibility_shading_render_pipeline(&mut app, |task| loading_tasks.push(task));

	let graph = locomotion_graph();
	let (mut pool, animation_worker) = AnimationPool::new(
		app.resource_manager_handle(),
		AnimationPoolConfig::new(NonZeroUsize::new(ANIMATION_POOL_BYTES).expect("the pool budget is non-zero")),
	);

	// The graph needs a target skeleton before it can evaluate. Load the mesh on
	// the same deferred runtime, then transfer only its skeleton back to the app.
	let (mesh_sender, mesh_receiver) = kanal::bounded_async(1);
	let mesh_receiver = mesh_receiver.to_sync();
	let resource_manager = app.resource_manager_handle();
	loading_tasks.push(Box::new(move |runtime| {
		runtime.spawn(animation_worker.run()).detach();
		runtime
			.spawn(async move {
				let mesh = resource_manager
					.request::<Mesh>(MODEL_RESOURCE)
					.await
					.map(|reference| reference.into_resource());
				let _ = mesh_sender.send(mesh).await;
			})
			.detach();
	}));
	byte_engine::application::graphics::defaults::launch_deferred_tasks_thread(&mut app, loading_tasks);

	let animated_handle = create_scene(&mut app);
	let mut player = None;
	let mut root_position = Point::origin();

	while app
		.tick_with(|app, time| {
			// Wait without blocking rendering until the target skeleton arrives. The
			// owned constructor lets this player retain that skeleton safely.
			if player.is_none() {
				match mesh_receiver.try_recv_realtime() {
					Ok(Some(Ok(mesh))) => {
						let skeleton = mesh
							.skeleton
							.expect("the model resource must contain a skeleton")
							.into_resource();
						player = Some(
							AnimationGraphPlayer::new_owned(&graph, skeleton, ROOT_MOTION_NODE)
								.expect("the configured root node must exist in the model skeleton"),
						);
					}
					Ok(Some(Err(error))) => panic!("Unable to load '{MODEL_RESOURCE}': {error}"),
					Ok(None) => return,
					Err(_) => panic!("The mesh loader closed before returning '{MODEL_RESOURCE}'"),
				}
			}

			let Some(player) = player.as_mut() else {
				return;
			};
			// Replace this timed input with your input, AI, or networking state. It
			// deliberately alternates every two seconds to exercise both transitions.
			let input = LocomotionInput {
				moving: (time.elapsed().as_seconds_f32() as u32 / 2).is_multiple_of(2),
			};
			let pose = player
				.advance(time.delta(), &input, &mut pool)
				.expect("application frame time is never negative");

			// Apply root motion before submitting the pose. The simple harness moves
			// only translation; turning clips should also compose `root_motion.rotation`
			// with the object's orientation using the application's transform convention.
			let root_motion = pose.root_motion();
			root_position = root_position
				+ Vector::new(
					root_motion.translation[0],
					root_motion.translation[1],
					root_motion.translation[2],
				);

			let world = app.world_mut();
			world.transforms_channel_mut().send(TransformationUpdate::new(
				animated_handle,
				Transform::from_position(root_position),
			));
			// `UpdatePose` crosses into renderer-owned state, so it owns a matrix
			// vector. Graph evaluation itself continues to reuse its retained buffers.
			world
				.poses_channel_mut()
				.send(UpdatePose::new(animated_handle, pose.global_pose().to_vec()));
		})
		.is_some()
	{}
}

/// Creates the renderer-facing objects that receive root-motion and pose updates.
fn create_scene(app: &mut GraphicsApplication) -> byte_engine::core::factory::Handle {
	let mut camera = Camera::new();
	camera.set_position(Point::new(0.0, 1.5, 5.0));
	camera.set_direction(
		Vector::new(0.0, -0.15, -1.0)
			.normalized()
			.expect("camera direction is non-zero"),
	);
	let camera = app.world_mut().camera_factory_mut().create(camera);

	let mut window = Window::new("Animation Graph", Extent::rectangle(1280, 720));
	window.attach(camera);
	app.window_factory_mut().create(window);

	let object = Object::new(
		MODEL_RESOURCE,
		Transform::from_position(Point::origin()),
		byte_engine::physics::BodyTypes::Static,
		Vector::zero(),
	);
	let renderable = app.world_mut().renderable_factory_mut().create(EntityHandle::from(object));
	renderable
}
