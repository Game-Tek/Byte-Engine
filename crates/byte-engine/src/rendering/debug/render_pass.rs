/// The `DebugMeshRenderPass` struct composites wireframe diagnostic meshes over one sink's current color image.
///
/// Register it through [`crate::application::graphics::setup_debug_mesh_render_pass`].
/// Register that setup after tone mapping, all other post-processing, and any
/// overlay it should cover to keep debug colors unchanged and on top. The pass
/// can also run before tone mapping when post-processed, scene-linear debug
/// colors are useful.
pub struct DebugMeshRenderPass {
	scene: Rc<RefCell<DebugSceneManager>>,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	depth_pipeline: crate::rendering::PipelineRef,
	overlay_pipeline: crate::rendering::PipelineRef,
	working_color: ghi::BaseImageHandle,
	depth: ghi::BaseImageHandle,
	source_copy: ImageBypassPass,
	output_copy: ImageBypassPass,
	bypass_copy: ImageBypassPass,
}

impl DebugMeshRenderPass {
	/// Creates one sink-local compositing pass backed by the shared debug scene.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>, scene: Rc<RefCell<DebugSceneManager>>) -> Self {
		let source = render_pass_builder.read_from("main");
		let depth = render_pass_builder.read_from("depth");
		// Raster into a sampleable image even when this pass is terminal because a
		// swapchain cannot be both a raster attachment and the generic copy output.
		let working_color = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				crate::rendering::SCENE_COLOR_FORMAT,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage,
			)
			.name("Debug Mesh Working"),
		);
		let output = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(crate::rendering::SCENE_COLOR_FORMAT, ghi::Uses::Storage | ghi::Uses::Image)
				.name("Debug Meshes"),
		);

		let pipeline_manager = render_pass_builder.pipeline_manager().clone();
		let depth_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/debug/depth.pipeline");
		let overlay_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/debug/overlay.pipeline");
		let working_color_handle = ghi::BaseImageHandle::from(working_color);
		let source_copy = ImageBypassPass::new(render_pass_builder, source, working_color_handle);
		let output_copy = ImageBypassPass::new(render_pass_builder, working_color_handle, output);
		let bypass_copy = ImageBypassPass::new(render_pass_builder, source, output);

		Self {
			scene,
			pipeline_manager,
			depth_pipeline,
			overlay_pipeline,
			working_color: working_color_handle,
			depth: depth.into(),
			source_copy,
			output_copy,
			bypass_copy,
		}
	}

	/// Expands the shared frame scene into sink-specific matrices and separates depth modes.
	fn prepare_draws<'a>(
		&self,
		frame: &ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> (&'a [PreparedDraw], &'a [PreparedDraw]) {
		let mut scene = self.scene.borrow_mut();
		let sphere = scene.mesh(MeshKind::Sphere);
		let r#box = scene.mesh(MeshKind::Box);
		let cylinder = scene.mesh(MeshKind::Cylinder);

		let view_projection = sink.view_projection();
		let mut depth_draws = bumpalo::collections::Vec::new_in(frame_allocator);
		let mut overlay_draws = bumpalo::collections::Vec::new_in(frame_allocator);
		for debug_mesh in scene.debug_meshes(frame.key()) {
			let target = match debug_mesh.selected_depth_mode() {
				DebugDepthMode::Scene => &mut depth_draws,
				DebugDepthMode::Ignore => &mut overlay_draws,
			};
			let color = debug_mesh.color();
			for_each_shape_instance(debug_mesh.shape(), |kind, model| {
				let mesh = match kind {
					MeshKind::Sphere => sphere,
					MeshKind::Box => r#box,
					MeshKind::Cylinder => cylinder,
				};
				target.push(PreparedDraw {
					mesh,
					push_constants: DebugPushConstants {
						view_projection_model: (view_projection * model).into(),
						color: [color.r, color.g, color.b, color.a],
					},
				});
			});
		}

		(depth_draws.into_bump_slice(), overlay_draws.into_bump_slice())
	}

	/// Applies this frame's pending retained-mesh changes even when the pass is bypassed.
	fn update_scene(&self, frame: &ghi::implementation::Frame) {
		self.scene.borrow_mut().debug_meshes(frame.key()).for_each(drop);
	}
}

impl RenderPass for DebugMeshRenderPass {
	fn name(&self) -> &'static str {
		"debug-meshes"
	}

	/// Prepares copies and wireframe draws while preserving the incoming color and scene depth images.
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let (depth_draws, overlay_draws) = self.prepare_draws(frame, sink, frame_allocator);
		if depth_draws.is_empty() && overlay_draws.is_empty() {
			return self.bypass_copy.prepare(frame, sink, frame_allocator);
		}

		let depth_pipeline = if depth_draws.is_empty() {
			None
		} else if let Some(pipeline) = self.pipeline_manager.pipeline(self.depth_pipeline) {
			Some(pipeline)
		} else {
			return self.bypass_copy.prepare(frame, sink, frame_allocator);
		};
		let overlay_pipeline = if overlay_draws.is_empty() {
			None
		} else if let Some(pipeline) = self.pipeline_manager.pipeline(self.overlay_pipeline) {
			Some(pipeline)
		} else {
			return self.bypass_copy.prepare(frame, sink, frame_allocator);
		};
		let Some(source_copy) = self.source_copy.prepare(frame, sink, frame_allocator) else {
			return self.bypass_copy.prepare(frame, sink, frame_allocator);
		};
		let Some(output_copy) = self.output_copy.prepare(frame, sink, frame_allocator) else {
			return self.bypass_copy.prepare(frame, sink, frame_allocator);
		};
		let extent = sink.extent();
		let working_color = self.working_color;
		let depth = self.depth;

		Some(allocate_render_command(frame_allocator, move |command_buffer, _| {
			command_buffer.region(
				|label| label.write_str("Debug Meshes"),
				|command_buffer| {
					source_copy(command_buffer, &[]);

					if let Some(pipeline) = depth_pipeline {
						let attachments = [
							ghi::AttachmentInformation::new(
								working_color,
								ghi::Layouts::RenderTarget,
								ghi::ClearValue::None,
								true,
								true,
							),
							// Load scene depth for testing but never store it: depth-aware debug
							// geometry reads the scene pipeline's `depth` image without modifying it.
							ghi::AttachmentInformation::new(
								depth,
								ghi::Layouts::RenderTarget,
								ghi::ClearValue::None,
								true,
								false,
							),
						];
						let command_buffer = command_buffer.start_render_pass(extent, &attachments);
						let command_buffer = command_buffer.bind_raster_pipeline(pipeline);
						for draw in depth_draws {
							command_buffer.write_push_constant(0, draw.push_constants);
							command_buffer.draw_mesh(&draw.mesh);
						}
						command_buffer.end_render_pass();
					}

					// Depth-ignored geometry is recorded last so it also overlays depth-aware diagnostics.
					if let Some(pipeline) = overlay_pipeline {
						let attachments = [ghi::AttachmentInformation::new(
							working_color,
							ghi::Layouts::RenderTarget,
							ghi::ClearValue::None,
							true,
							true,
						)];
						let command_buffer = command_buffer.start_render_pass(extent, &attachments);
						let command_buffer = command_buffer.bind_raster_pipeline(pipeline);
						for draw in overlay_draws {
							command_buffer.write_push_constant(0, draw.push_constants);
							command_buffer.draw_mesh(&draw.mesh);
						}
						command_buffer.end_render_pass();
					}

					output_copy(command_buffer, &[]);
				},
			);
		}))
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.update_scene(frame);
		self.bypass_copy.prepare(frame, sink, frame_allocator)
	}
}

/// Visits the one to three unit meshes that form a semantic debug shape.
fn for_each_shape_instance(shape: DebugShape, mut visit: impl FnMut(MeshKind, Matrix)) {
	match shape {
		DebugShape::Sphere { center, radius } => visit(
			MeshKind::Sphere,
			model_matrix(
				center,
				Orientation::identity().into_matrix(),
				Vec3f::new(radius, radius, radius),
			),
		),
		DebugShape::Box {
			center,
			half_extents,
			orientation,
		} => visit(
			MeshKind::Box,
			model_matrix(center, orientation.into_matrix(), half_extents.into_maths()),
		),
		DebugShape::Capsule { start, end, radius } => {
			let axis = end - start;
			if let Ok((direction, length)) = axis.normalize_with_length() {
				visit(
					MeshKind::Sphere,
					model_matrix(
						start,
						Orientation::identity().into_matrix(),
						Vec3f::new(radius, radius, radius),
					),
				);
				visit(
					MeshKind::Cylinder,
					model_matrix(
						start + axis * 0.5,
						orientation_from_direction(direction).into_matrix(),
						Vec3f::new(radius, radius, length * 0.5),
					),
				);
				visit(
					MeshKind::Sphere,
					model_matrix(end, Orientation::identity().into_matrix(), Vec3f::new(radius, radius, radius)),
				);
			} else {
				// A zero-length capsule is still a well-defined sphere.
				visit(
					MeshKind::Sphere,
					model_matrix(
						start,
						Orientation::identity().into_matrix(),
						Vec3f::new(radius, radius, radius),
					),
				);
			}
		}
		DebugShape::Segment { start, end } => {
			let axis = end - start;
			let (direction, length) = axis.normalize_with_length().expect(
				"Debug segment direction is invalid. The most likely cause is that shape expansion ran before message validation.",
			);
			visit(
				MeshKind::Cylinder,
				model_matrix(
					start + axis * 0.5,
					orientation_from_direction(direction).into_matrix(),
					Vec3f::new(SEGMENT_RADIUS, SEGMENT_RADIUS, length * 0.5),
				),
			);
		}
	}
}

/// Builds the scale-then-rotate-then-translate matrix used by each unit debug mesh.
fn model_matrix(center: Point, rotation: Matrix, scale: Vec3f) -> Matrix {
	Matrix::from_translation(center.into_maths()) * rotation * Matrix::from_scale(scale)
}

const SEGMENT_RADIUS: f32 = 0.005;

/// The `DebugPushConstants` struct packs one transformed mesh and straight-alpha color for BESL.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DebugPushConstants {
	view_projection_model: ShaderMatrix,
	color: [f32; 4],
}

/// The `PreparedDraw` struct keeps one frame-local mesh draw ready for command recording.
#[derive(Clone, Copy)]
struct PreparedDraw {
	mesh: ghi::MeshHandle,
	push_constants: DebugPushConstants,
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, Value, builtin_position_slot, input_slot, output_slot};
	use math::{Orientation, Point, Vector};
	use maths_rs::Vec4f;

	use super::*;
	use crate::rendering::shader_vm_test::{
		builtin_position_buffer, compile, input_buffer, output_buffer, push_constant_buffer, run_at,
	};

	const DEBUG_VERTEX_BESL: &str = include_str!("../../../assets/rendering/debug/vertex.besl");
	const DEBUG_FRAGMENT_BESL: &str = include_str!("../../../assets/rendering/debug/fragment.besl");
	const IDENTITY_MATRIX: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

	/// Links one checked-in debug shader through the same BESL frontend used by production baking.
	fn debug_program(source: &str, name: &str) -> besl::NodeReference {
		besl::compile_to_besl(source, None)
			.unwrap_or_else(|error| {
				panic!(
					"Failed to link {name}: {error:?}. The most likely cause is invalid syntax in the checked-in debug shader."
				)
			})
			.get_main()
			.unwrap_or_else(|| {
				panic!(
					"Missing {name} entry point. The most likely cause is that the checked-in debug shader has no `main` function."
				)
			})
	}

	/// Collects only mesh kinds so semantic expansion remains independent of native GHI handles.
	fn mesh_kinds(shape: DebugShape) -> Vec<MeshKind> {
		shape_instances(shape).into_iter().map(|(mesh, _)| mesh).collect()
	}

	/// Collects semantic mesh transforms without constructing native GHI resources.
	fn shape_instances(shape: DebugShape) -> Vec<(MeshKind, Matrix)> {
		let mut instances = Vec::new();
		for_each_shape_instance(shape, |mesh, transform| instances.push((mesh, transform)));
		instances
	}

	/// Verifies one local mesh position maps to the expected world-space point.
	fn assert_transformed_point(transform: Matrix, local: [f32; 3], expected: Point) {
		let actual = transform * Vec4f::new(local[0], local[1], local[2], 1.0);
		assert!(
			(actual.x - expected.x()).abs() < 1e-5
				&& (actual.y - expected.y()).abs() < 1e-5
				&& (actual.z - expected.z()).abs() < 1e-5
				&& (actual.w - 1.0).abs() < 1e-5,
			"Debug mesh transform produced {actual:?}, expected {expected:?}. The most likely cause is an incorrect scale, orientation, or center."
		);
	}

	/// Verifies every public shape selects the unit meshes needed for its visible geometry.
	#[test]
	fn supported_shapes_expand_into_the_expected_unit_meshes() {
		assert_eq!(
			mesh_kinds(DebugShape::Sphere {
				center: Point::origin(),
				radius: 1.0,
			}),
			vec![MeshKind::Sphere]
		);
		assert_eq!(
			mesh_kinds(DebugShape::Box {
				center: Point::origin(),
				half_extents: Vector::new(1.0, 2.0, 3.0),
				orientation: Orientation::identity(),
			}),
			vec![MeshKind::Box]
		);
		assert_eq!(
			mesh_kinds(DebugShape::Capsule {
				start: Point::origin(),
				end: Point::new(0.0, 2.0, 0.0),
				radius: 0.5,
			}),
			vec![MeshKind::Sphere, MeshKind::Cylinder, MeshKind::Sphere]
		);
		assert_eq!(
			mesh_kinds(DebugShape::Segment {
				start: Point::origin(),
				end: Point::new(0.0, 0.0, 2.0),
			}),
			vec![MeshKind::Cylinder]
		);
	}

	#[test]
	fn zero_length_capsule_expands_to_one_sphere() {
		assert_eq!(
			mesh_kinds(DebugShape::Capsule {
				start: Point::new(1.0, 2.0, 3.0),
				end: Point::new(1.0, 2.0, 3.0),
				radius: 2.0,
			}),
			vec![MeshKind::Sphere]
		);
	}

	/// Confirms +Z unit cylinders span the caller's world-space endpoints after expansion.
	#[test]
	fn capsule_and_segment_cylinders_follow_their_endpoints() {
		let capsule_start = Point::new(-2.0, 1.0, 3.0);
		let capsule_end = Point::new(4.0, 5.0, -1.0);
		let capsule = shape_instances(DebugShape::Capsule {
			start: capsule_start,
			end: capsule_end,
			radius: 0.75,
		});
		assert_eq!(capsule[1].0, MeshKind::Cylinder);
		assert_transformed_point(capsule[1].1, [0.0, 0.0, -1.0], capsule_start);
		assert_transformed_point(capsule[1].1, [0.0, 0.0, 1.0], capsule_end);

		let segment_start = Point::new(2.0, -3.0, 7.0);
		let segment_end = Point::new(-1.0, 6.0, 2.0);
		let segment = shape_instances(DebugShape::Segment {
			start: segment_start,
			end: segment_end,
		});
		assert_eq!(segment[0].0, MeshKind::Cylinder);
		assert_transformed_point(segment[0].1, [0.0, 0.0, -1.0], segment_start);
		assert_transformed_point(segment[0].1, [0.0, 0.0, 1.0], segment_end);
	}

	#[test]
	fn debug_push_constants_match_the_persisted_pipeline_range() {
		assert_eq!(std::mem::size_of::<DebugPushConstants>(), 80);
		assert_eq!(std::mem::align_of::<DebugPushConstants>(), 16);
	}

	#[test]
	fn checked_in_debug_raster_shaders_link() {
		debug_program(DEBUG_VERTEX_BESL, "debug vertex shader");
		debug_program(DEBUG_FRAGMENT_BESL, "debug fragment shader");
	}

	/// Executes both checked-in stages through the BESL VM to verify the raster interface and color path.
	#[test]
	fn debug_vertex_and_fragment_vm_preserve_position_and_color() {
		let vertex = compile(debug_program(DEBUG_VERTEX_BESL, "debug vertex shader"));
		let mut push_constant = push_constant_buffer(&vertex);
		let mut vertex_input = input_buffer(&vertex, 0);
		let mut position = builtin_position_buffer(&vertex);
		let mut vertex_color = output_buffer(&vertex, 0);
		push_constant
			.write("view_projection_model", Value::Mat4F(IDENTITY_MATRIX))
			.expect("Failed to seed the debug transform. The most likely cause is a changed push-constant interface.");
		push_constant
			.write("color", Value::Vec4F([0.1, 0.2, 0.3, 0.4]))
			.expect("Failed to seed the debug color. The most likely cause is a changed push-constant interface.");
		vertex_input
			.write("in_position", Value::Vec3F([1.0, 2.0, 3.0]))
			.expect("Failed to seed the debug position. The most likely cause is a changed vertex interface.");
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_push_constant(&mut push_constant);
			descriptors.bind_buffer(input_slot(0), &mut vertex_input);
			descriptors.bind_buffer(builtin_position_slot(), &mut position);
			descriptors.bind_buffer(output_slot(0), &mut vertex_color);
			run_at(&vertex, &mut descriptors, [0, 0]);
		}
		assert_eq!(
			position.read("_besl_interface_position"),
			Ok(Value::Vec4F([1.0, 2.0, 3.0, 1.0]))
		);
		assert_eq!(
			vertex_color.read("_besl_interface_color"),
			Ok(Value::Vec4F([0.1, 0.2, 0.3, 0.4]))
		);

		let fragment = compile(debug_program(DEBUG_FRAGMENT_BESL, "debug fragment shader"));
		let mut fragment_color = input_buffer(&fragment, 0);
		let mut output = output_buffer(&fragment, 0);
		fragment_color
			.write("_besl_interface_color", Value::Vec4F([0.1, 0.2, 0.3, 0.4]))
			.expect("Failed to seed the fragment color. The most likely cause is a changed raster interface.");
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot(0), &mut fragment_color);
			descriptors.bind_buffer(output_slot(0), &mut output);
			run_at(&fragment, &mut descriptors, [0, 0]);
		}
		assert_eq!(
			output.read("_besl_output_color_attachment"),
			Ok(Value::Vec4F([0.1, 0.2, 0.3, 0.4]))
		);
	}
}

use std::{cell::RefCell, rc::Rc};

use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	frame::Frame as _,
};
use math::{Matrix, Orientation, Point, ShaderMatrix, orientation_from_direction};
use maths_rs::{
	Vec3f,
	mat::{MatScale as _, MatTranslate as _},
};

use super::scene_manager::MeshKind;
use crate::rendering::{
	Sink,
	debug::{DebugDepthMode, DebugSceneManager, DebugShape},
	render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, allocate_render_command},
	render_passes::blit::ImageBypassPass,
};
