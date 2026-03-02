use ::utils::Extent;

use crate::{camera::Camera, gameplay::Positionable as _};

pub mod common_shader_generator;

pub mod lights;
pub mod window;

pub mod mesh;

pub mod renderable;

pub mod cct;

pub mod scene_manager;
pub mod world_render_domain;

pub mod renderer;
pub mod texture_manager;

pub mod framebuffer;
pub mod render_pass;
pub mod render_passes;

pub mod pipeline_manager;

pub mod pipelines;

pub mod view;
pub mod viewport;

pub mod csm;

pub mod utils;

pub use render_pass::RenderPass;
pub use renderable::mesh::RenderableMesh;
pub use view::View;
pub use viewport::Viewport;

/// Maps a shader resource binding to a GHI shader binding descriptor.
pub fn map_shader_binding_to_shader_binding_descriptor(
	b: &resource_management::spirv_shader_generator::Binding,
) -> ghi::ShaderBindingDescriptor {
	ghi::ShaderBindingDescriptor::new(
		b.set,
		b.binding,
		if b.read {
			ghi::AccessPolicies::READ
		} else {
			ghi::AccessPolicies::empty()
		} | if b.write {
			ghi::AccessPolicies::WRITE
		} else {
			ghi::AccessPolicies::empty()
		},
	)
}

pub fn make_perspective_view_from_camera(camera: &Camera, extent: Extent) -> View {
	let (camera_position, camera_orientation, fov_y) = (camera.position(), camera.get_direction(), camera.get_fov());

	let aspect_ratio = extent.width() as f32 / extent.height() as f32;

	View::new_perspective(fov_y, aspect_ratio, 0.1f32, 100f32, camera_position, camera_orientation)
}
