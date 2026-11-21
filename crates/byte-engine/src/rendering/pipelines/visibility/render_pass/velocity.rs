//! Velocity: where each pixel's surface point showed in the previous frame, which temporal passes reproject through.
//!
//! [`VelocityPass`] writes the `Velocity` target from the opaque layer, after the depth pyramid and before the
//! transparent layer reuses the visibility buffer. It runs while an enabled pass of the sink reads `Velocity`, such as
//! temporal antialiasing or motion blur; see the temporal antialiasing design note.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use math::{Matrix, inverse};
use utils::Extent;

use super::{ComputeStage, Pipelines, SinkHistory};
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::{PipelineManagerClient, Sink, TargetId, View};

/// The velocity and model store pipelines, named as [`Pipelines::request`] takes them.
const VELOCITY_PIPELINES: [&str; 2] = ["velocity", "velocity-models"];

/// The name of the target the velocity stage writes, which temporal passes read.
pub(crate) const VELOCITY_TARGET: &str = "Velocity";

/// The name of the reactive mask, which the velocity stage clears and blended surfaces raise.
pub(crate) const REACTIVE_TARGET: &str = "Reactive";

/// The `VelocityTargets` struct holds the images the velocity stage writes, which temporal passes read by name.
#[derive(Clone, Copy)]
pub(crate) struct VelocityTargets {
	/// Each pixel's motion and its opaque surface's previous and current depth, named [`VELOCITY_TARGET`], with a copy
	/// per frame in flight so temporal passes read the previous frame's depth.
	pub(crate) velocity: ghi::BaseImageHandle,
	/// How little temporal passes trust their history at each pixel, named [`REACTIVE_TARGET`].
	pub(crate) reactive: ghi::BaseImageHandle,
	/// The velocity target's identity, which tells whether an enabled pass reads it; see [`Sink::reads`].
	pub(crate) id: TargetId,
}

/// Creates the velocity stage's targets for the sink that `render_pass_builder` sets up.
pub(crate) fn create_velocity_targets(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> VelocityTargets {
	let uses = ghi::Uses::Storage | ghi::Uses::Image;
	let reactive =
		render_pass_builder.create_render_target(ghi::image::Builder::new(ghi::Formats::R8UNORM, uses).name(REACTIVE_TARGET));
	// Temporal passes read the previous frame's opaque depth from its alpha.
	let velocity = render_pass_builder
		.create_history_target(ghi::image::Builder::new(ghi::Formats::RGBA16F, uses).name(VELOCITY_TARGET), 1);
	VelocityTargets {
		velocity: velocity.into(),
		reactive: reactive.into(),
		id: render_pass_builder.target_id(VELOCITY_TARGET),
	}
}

/// The `VelocityShaderParameters` struct carries one frame's camera constants, as `VelocityParameters` in
/// `layouts/velocity.besl` declares them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct VelocityShaderParameters {
	pub(crate) clip_to_world: ghi::pod::Mat4f,
	pub(crate) current_view_projection: ghi::pod::Mat4f,
	pub(crate) previous_view_projection: ghi::pod::Mat4f,
	pub(crate) background_to_previous_clip: ghi::pod::Mat4f,
	pub(crate) jitter: [f32; 2],
	pub(crate) instance_capacity: u32,
	pub(crate) far: f32,
	/// `1` when the previous frame's model store ran, so the previous models hold that frame's; `0` makes every
	/// instance stand where it stands now.
	pub(crate) previous_models_valid: u32,
	pub(crate) _padding: [u32; 3],
}

/// The `VelocityPass` struct writes a sink's per-pixel motion while an enabled pass reads it, so temporal passes find
/// where each pixel's surface was in the previous frame and motion blur smears it along its motion.
pub(super) struct VelocityPass {
	/// The scene set, then the velocity set.
	descriptor_sets: [ghi::DescriptorSetHandle; 2],
	parameters: ghi::BufferHandle<VelocityShaderParameters>,
	instance_capacity: u32,
	/// The velocity target's identity, which the sink reports readers of.
	target: TargetId,
	pub(super) pipelines: Pipelines<2>,
}

impl VelocityPass {
	/// Binds the opaque layer's `depth` and visibility buffer, the skinned vertices' `previous_vertex_positions`, and
	/// the `targets` the pass writes. `instance_capacity` sizes the previous model matrices it keeps.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		scene_descriptor_set: ghi::DescriptorSetHandle,
		depth: ghi::BaseImageHandle,
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		previous_vertex_positions: ghi::BufferHandle<[[f32; 3]]>,
		targets: VelocityTargets,
		instance_capacity: u32,
	) -> Self {
		let descriptor_set = set_layouts::VELOCITY.create_set(context, "Velocity Descriptor Set");
		let parameters = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Velocity Parameters")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		// Only the GPU writes the previous models, so one copy serves every frame: each frame's velocity reads what the
		// previous frame's model store left before its own store replaces it.
		let previous_models = context.build_buffer::<[ghi::pod::Mat4x3f]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Velocity Previous Models")
				.length(instance_capacity as usize),
		);
		let binder = set_layouts::VELOCITY.binder(descriptor_set);
		context.write(&[
			binder.buffer("parameters", parameters),
			binder.texture("depth", depth),
			binder.image("triangle_index", primitive_index),
			binder.image("instance_index_render_target", instance_id),
			binder.buffer("previous_models", previous_models),
			binder.buffer("previous_vertex_positions", previous_vertex_positions),
			binder.image("velocity_output", targets.velocity),
			binder.image("reactive_output", targets.reactive),
		]);
		Self {
			descriptor_sets: [scene_descriptor_set, descriptor_set],
			parameters,
			instance_capacity,
			target: targets.id,
			pipelines: Pipelines::request(pipeline_manager, VELOCITY_PIPELINES),
		}
	}

	/// Reports whether an enabled pass of `sink` reads the velocity this frame, so the stage runs.
	pub(super) fn is_read(&self, sink: &Sink) -> bool {
		sink.reads(self.target)
	}

	/// Uploads this frame's camera constants and returns the velocity dispatch and the model store after it, or `None`
	/// while no enabled pass reads the velocity. `history` holds the previous frame's view, or is `None` when there's
	/// none, which gives the camera no motion. When the previous frame didn't run the stage, its model store kept models
	/// from an older frame, so every instance counts as still.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		history: Option<SinkHistory>,
		[velocity, models]: [ghi::PipelineHandle; 2],
	) -> Option<[ComputeStage<2>; 2]> {
		if !self.is_read(sink) {
			return None;
		}
		let previous = history.map_or(sink.view(), |history| history.view);
		let previous_models_valid = history.is_some_and(|history| history.read_targets.contains(self.target));
		*frame.map(self.parameters) = velocity_shader_parameters(sink, previous, self.instance_capacity, previous_models_valid);
		Some([
			ComputeStage {
				label: "Velocity",
				pipeline: velocity,
				descriptor_sets: self.descriptor_sets,
				extent: sink.extent(),
				workgroup: Extent::new(8, 8, 1),
			},
			ComputeStage {
				label: "Velocity Models",
				pipeline: models,
				descriptor_sets: self.descriptor_sets,
				extent: Extent::new(self.instance_capacity, 1, 1),
				workgroup: Extent::new(64, 1, 1),
			},
		])
	}
}

/// Builds the velocity stage's constants for `sink`, whose previous frame rendered from `previous`, without jitter.
/// `previous_models_valid` says whether the previous frame stored its models.
pub(crate) fn velocity_shader_parameters(
	sink: &Sink,
	previous: View,
	instance_capacity: u32,
	previous_models_valid: bool,
) -> VelocityShaderParameters {
	let jittered = sink.jittered_view();
	VelocityShaderParameters {
		clip_to_world: inverse(jittered.view_projection()).into(),
		current_view_projection: sink.view().view_projection().into(),
		previous_view_projection: previous.view_projection().into(),
		background_to_previous_clip: background_to_previous_clip(jittered, previous).into(),
		jitter: sink.jitter(),
		instance_capacity,
		far: sink.view().far(),
		previous_models_valid: previous_models_valid as u32,
		_padding: [0; 3],
	}
}

/// Returns the transform from `current`'s normalized device coordinates at depth 0 to the clip space of `previous`,
/// both with their translation removed, so background directions move only as the camera turns.
fn background_to_previous_clip(current: View, previous: View) -> Matrix {
	let rotation = |view: View| {
		let mut matrix = view.view();
		// The view's last column holds its translation; directions take none.
		for row in 0..3 {
			matrix[row * 4 + 3] = 0.0;
		}
		matrix
	};
	previous.projection() * rotation(previous) * inverse(current.projection() * rotation(current))
}

#[cfg(test)]
mod tests {
	use math::{Degrees, Point};
	use maths_rs::Vec4f;

	use super::*;

	/// A direction maps to where the previous camera shows it, whatever the camera's position.
	#[test]
	fn background_directions_follow_the_turn_and_ignore_the_move() {
		let view = |position: Point, direction: [f32; 3]| {
			View::new_perspective(
				Degrees::new(60.0),
				1.5,
				0.1,
				100.0,
				position,
				math::Vector::new(direction[0], direction[1], direction[2])
					.normalized()
					.unwrap(),
			)
		};
		let previous = view(Point::new(0.0, 0.0, 0.0), [0.0, 0.0, 1.0]);
		let current = view(Point::new(5.0, 1.0, -3.0), [0.2, 0.0, 1.0]);
		let world_direction = [0.3_f32, 0.1, 1.0];
		// Where each camera shows the direction, as a point far along it from that camera.
		let shown = |view: View, eye: [f32; 3]| {
			let far = Vec4f::new(
				eye[0] + world_direction[0] * 1.0e4,
				eye[1] + world_direction[1] * 1.0e4,
				eye[2] + world_direction[2] * 1.0e4,
				1.0,
			);
			let clip = view.view_projection() * far;
			[clip.x / clip.w, clip.y / clip.w]
		};
		let [x, y] = shown(current, [5.0, 1.0, -3.0]);
		let clip = background_to_previous_clip(current, previous) * Vec4f::new(x, y, 0.0, 1.0);
		let expected = shown(previous, [0.0, 0.0, 0.0]);
		assert!((clip.x / clip.w - expected[0]).abs() < 1e-3, "{clip:?} {expected:?}");
		assert!((clip.y / clip.w - expected[1]).abs() < 1e-3, "{clip:?} {expected:?}");
	}
}
