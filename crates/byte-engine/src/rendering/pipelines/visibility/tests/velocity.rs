//! The velocity stage's contract: each pixel's motion since the previous frame, without jitter, in pixels, and the
//! surface's previous linear depth. See the `Velocity` target in the rendering reference.

use besl::vm::{DescriptorBindings, Texture, Value};
use math::{Degrees, Point, UnitVector};
use utils::Extent;

use super::super::layout::ShaderMeshletData;
use super::super::render_pass::{VelocityShaderParameters, velocity_shader_parameters};
use super::{
	FIXTURE_TABLE_LENGTH, IDENTITY_AFFINE_MATRIX, MESH_DATA, MESH_LEVELS, MESHLETS, PRIMITIVE_INDICES, TILE_WORKGROUP_SIZE,
	TILE_WORKGROUP_WIDTH, VERTEX_INDICES, VERTEX_POSITIONS, asset_program, instance_texture, run_workgroup_containing,
	write_meshlet,
};
use crate::rendering::shader_vm_test::{array_buffer, asset_source, buffer, compile, empty_image, rgba, slot, texture_2d};
use crate::rendering::{Sink, View};

const EXTENT: u32 = 8;
/// The depth of the wall the fixture camera faces, in meters.
const WALL_Z: f32 = 2.0;

/// The fixture camera at the origin, looking along +Z.
fn camera() -> View {
	View::new_perspective(Degrees::new(60.0), 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis())
}

/// Where `view` shows the world point `point` on the fixture image, in pixels, x right and y down.
fn pixel_of(view: View, point: [f32; 3]) -> [f32; 2] {
	let clip = view.view_projection() * maths_rs::Vec4f::new(point[0], point[1], point[2], 1.0);
	let (x, y) = (clip.x / clip.w, clip.y / clip.w);
	[(x * 0.5 + 0.5) * EXTENT as f32, (0.5 - y * 0.5) * EXTENT as f32]
}

/// Flattens a shader `mat4f` into the column-major floats the VM writes.
fn matrix(value: ghi::pod::Mat4f) -> Value {
	let columns: [[f32; 4]; 4] = value.into();
	Value::Mat4F(bytemuck::cast(columns))
}

/// Returns the wall's affine model matrix, translated by `offset`.
fn translated(offset: [f32; 3]) -> [f32; 12] {
	let mut model = IDENTITY_AFFINE_MATRIX;
	model[9..12].copy_from_slice(&offset);
	model
}

/// Runs the velocity stage at `pixel` over a wall at [`WALL_Z`] that fills the image, drawn with the model translated by
/// `model_offset` this frame and by `previous_offset` in the previous frame, from `sink` whose previous frame rendered
/// from `previous`. A `None` previous offset leaves the previous model zero, as an instance's first frame has it.
fn run_velocity(
	sink: &Sink,
	previous: View,
	model_offset: [f32; 3],
	previous_offset: Option<[f32; 3]>,
	pixel: [u32; 2],
) -> [f32; 4] {
	run_velocity_after(sink, previous, model_offset, previous_offset, true, WALL_Z, pixel)
}

/// Runs [`run_velocity`] for a frame whose previous frame stored its models when `previous_models_valid` holds, and
/// skipped the velocity stage otherwise, with every pixel's depth at view depth `depth_z`.
fn run_velocity_after(
	sink: &Sink,
	previous: View,
	model_offset: [f32; 3],
	previous_offset: Option<[f32; 3]>,
	previous_models_valid: bool,
	depth_z: f32,
	pixel: [u32; 2],
) -> [f32; 4] {
	let builtins = besl::Builtins::new();
	let program = compile(&asset_program(&builtins, &asset_source("visibility/velocity.besl")));
	let parameters: VelocityShaderParameters =
		velocity_shader_parameters(sink, previous, FIXTURE_TABLE_LENGTH as u32, previous_models_valid);
	let mut parameter_buffer = buffer(&program, slot(&program, "parameters"));
	for (member, value) in [
		("clip_to_world", matrix(parameters.clip_to_world)),
		("current_view_projection", matrix(parameters.current_view_projection)),
		("previous_view_projection", matrix(parameters.previous_view_projection)),
		("background_to_previous_clip", matrix(parameters.background_to_previous_clip)),
		("jitter", Value::Vec2F(parameters.jitter)),
		("instance_capacity", Value::U32(parameters.instance_capacity)),
		("far", Value::F32(parameters.far)),
		("previous_models_valid", Value::U32(parameters.previous_models_valid)),
	] {
		parameter_buffer.write(member, value).expect("velocity parameters");
	}

	// One triangle, far wider than the view, on the plane z = WALL_Z in object space.
	let vertices = [[-50.0, -50.0, WALL_Z], [50.0, -50.0, WALL_Z], [0.0, 50.0, WALL_Z]];
	let mut positions = array_buffer(&program, slot(&program, VERTEX_POSITIONS), vertices.len());
	let mut vertex_indices = array_buffer(&program, slot(&program, VERTEX_INDICES), vertices.len());
	let mut primitive_indices = array_buffer(&program, slot(&program, PRIMITIVE_INDICES), vertices.len());
	for (index, vertex) in vertices.into_iter().enumerate() {
		positions
			.write_array_element(index, Value::Vec3F(vertex))
			.expect("wall vertex");
		vertex_indices
			.write_array_element(index, Value::U16(index as u16))
			.expect("vertex index");
		primitive_indices
			.write_array_element(index, Value::U8(index as u8))
			.expect("triangle index");
	}
	let mut meshlets = array_buffer(&program, slot(&program, MESHLETS), 1);
	write_meshlet(
		&mut meshlets,
		0,
		ShaderMeshletData::new(0, 0, [3, 1], [0.0; 4], [0.0; 4], [0.0, 0.0, -1.0], 2.0),
	);
	let mut mesh_levels = array_buffer(&program, slot(&program, MESH_LEVELS), 1);
	let mut meshes = array_buffer(&program, slot(&program, MESH_DATA), FIXTURE_TABLE_LENGTH);
	meshes
		.write_array_member(0, "model", Value::Mat4x3F(translated(model_offset)))
		.expect("mesh model matrix");
	let mut previous_models = array_buffer(&program, slot(&program, "previous_models"), FIXTURE_TABLE_LENGTH);
	if let Some(offset) = previous_offset {
		previous_models
			.write_array_element(0, Value::Mat4x3F(translated(offset)))
			.expect("previous model");
	}
	let mut previous_positions = array_buffer(&program, slot(&program, "previous_vertex_positions"), vertices.len());

	// The wall moves along x and y only, so its depth stays WALL_Z, and the jittered view gives every pixel the same
	// device depth.
	let device_depth = {
		let clip = sink.jittered_view().view_projection() * maths_rs::Vec4f::new(0.0, 0.0, depth_z, 1.0);
		clip.z / clip.w
	};
	let mut depth = texture_2d(
		EXTENT,
		EXTENT,
		&vec![[device_depth, 0.0, 0.0, 1.0]; (EXTENT * EXTENT) as usize],
	);
	let mut triangle_indices = Texture::new(EXTENT, EXTENT).expect("triangle index fixture");
	let mut instance_indices = instance_texture(EXTENT, |_| 0);
	let mut output = empty_image(EXTENT, EXTENT);
	let mut reactive = empty_image(EXTENT, EXTENT);

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(&program, "parameters"), &mut parameter_buffer);
	descriptors.bind_buffer(slot(&program, MESH_DATA), &mut meshes);
	descriptors.bind_buffer(slot(&program, MESHLETS), &mut meshlets);
	descriptors.bind_buffer(slot(&program, MESH_LEVELS), &mut mesh_levels);
	descriptors.bind_buffer(slot(&program, VERTEX_POSITIONS), &mut positions);
	descriptors.bind_buffer(slot(&program, VERTEX_INDICES), &mut vertex_indices);
	descriptors.bind_buffer(slot(&program, PRIMITIVE_INDICES), &mut primitive_indices);
	descriptors.bind_buffer(slot(&program, "previous_models"), &mut previous_models);
	descriptors.bind_buffer(slot(&program, "previous_vertex_positions"), &mut previous_positions);
	descriptors.bind_texture(slot(&program, "depth"), &mut depth);
	descriptors.bind_image(slot(&program, "triangle_index"), &mut triangle_indices);
	descriptors.bind_image(slot(&program, "instance_index_render_target"), &mut instance_indices);
	descriptors.bind_image(slot(&program, "velocity_output"), &mut output);
	descriptors.bind_image(slot(&program, "reactive_output"), &mut reactive);
	run_workgroup_containing::<TILE_WORKGROUP_SIZE>(&program, descriptors, TILE_WORKGROUP_WIDTH, pixel);
	rgba(&output, pixel)
}

fn sink(jitter: [f32; 2]) -> Sink {
	Sink::new(camera(), Extent::rectangle(EXTENT, EXTENT), 0).with_jitter(jitter)
}

/// A still camera and a still wall give no motion, whatever the jitter, and the wall's previous depth.
#[test]
fn a_still_scene_has_no_velocity_under_any_jitter() {
	for jitter in [[0.25, -0.125], [-0.375, 0.4]] {
		let velocity = run_velocity(&sink(jitter), camera(), [0.0; 3], Some([0.0; 3]), [3, 5]);
		assert!(
			velocity[0].abs() < 1e-3 && velocity[1].abs() < 1e-3,
			"{jitter:?}: {velocity:?}"
		);
		assert!(
			(velocity[2] - WALL_Z).abs() < 1e-3 && (velocity[3] - WALL_Z).abs() < 1e-3,
			"{velocity:?}"
		);
	}
}

/// A wall that moved 0.1 m right since the previous frame moves each pixel by where the camera shows that step.
#[test]
fn a_moving_wall_moves_its_pixels_by_its_projected_step() {
	let pixel = [4, 4];
	let velocity = run_velocity(&sink([0.2, 0.3]), camera(), [0.1, 0.0, 0.0], Some([0.0; 3]), pixel);
	// The surface point under the pixel, found without jitter, and where it stood in the previous frame.
	let step = pixel_of(camera(), [0.1, 0.0, WALL_Z])[0] - pixel_of(camera(), [0.0, 0.0, WALL_Z])[0];
	assert!(step > 0.1, "{step}");
	assert!((velocity[0] - step).abs() < 1e-3, "expected {step}, found {velocity:?}");
	assert!(velocity[1].abs() < 1e-3, "{velocity:?}");
}

/// An instance's first frame, which has no previous model yet, stands where it stands now. After a frame that skipped
/// the stage, the stored models are an older frame's, so a moved wall counts as still.
#[test]
fn an_instance_without_a_previous_model_has_no_velocity() {
	let first_frame = run_velocity(&sink([0.2, 0.3]), camera(), [0.1, 0.0, 0.0], None, [4, 4]);
	let after_skip = run_velocity_after(
		&sink([0.0, 0.0]),
		camera(),
		[0.1, 0.0, 0.0],
		Some([0.0; 3]),
		false,
		WALL_Z,
		[4, 4],
	);
	for velocity in [first_frame, after_skip] {
		assert!(velocity[0].abs() < 1e-3 && velocity[1].abs() < 1e-3, "{velocity:?}");
	}
}

/// A still wall seen from a moving camera moves by its own points' motion, also where a pixel's depth misses the wall's
/// plane, as it does at a triangle's edge or through rounding on a thin triangle.
#[test]
fn depth_off_the_triangle_takes_the_triangle_motion() {
	let previous = View::new_perspective(
		Degrees::new(60.0),
		1.0,
		0.1,
		100.0,
		Point::new(0.1, 0.0, 0.0),
		UnitVector::z_axis(),
	);
	// The camera's step moves a pixel by the wall's projected step whatever depth the pixel holds.
	let pixel = [1, 6];
	let on_plane = run_velocity_after(&sink([0.0, 0.0]), previous, [0.0; 3], Some([0.0; 3]), true, WALL_Z, pixel);
	let off_plane = run_velocity_after(
		&sink([0.0, 0.0]),
		previous,
		[0.0; 3],
		Some([0.0; 3]),
		true,
		WALL_Z * 3.0,
		pixel,
	);
	assert!(
		(on_plane[0] - off_plane[0]).abs() < 1e-3 && (on_plane[1] - off_plane[1]).abs() < 1e-3,
		"{on_plane:?} {off_plane:?}"
	);
	let still = run_velocity_after(
		&sink([0.0, 0.0]),
		camera(),
		[0.0; 3],
		Some([0.0; 3]),
		true,
		WALL_Z * 3.0,
		pixel,
	);
	assert!(still[0].abs() < 1e-3 && still[1].abs() < 1e-3, "{still:?}");
}

/// A camera that stepped 0.1 m left moves a still wall's pixels right by the same projected step.
#[test]
fn a_moving_camera_moves_still_pixels() {
	let previous = View::new_perspective(
		Degrees::new(60.0),
		1.0,
		0.1,
		100.0,
		Point::new(0.1, 0.0, 0.0),
		UnitVector::z_axis(),
	);
	let velocity = run_velocity(&sink([0.0, 0.0]), previous, [0.0; 3], Some([0.0; 3]), [4, 4]);
	let step = pixel_of(camera(), [0.1, 0.0, WALL_Z])[0] - pixel_of(camera(), [0.0, 0.0, WALL_Z])[0];
	assert!((velocity[0] - step).abs() < 1e-3, "expected {step}, found {velocity:?}");
}
