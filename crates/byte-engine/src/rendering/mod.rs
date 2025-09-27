use ::utils::Extent;

use crate::{camera::Camera, rendering::view::View};

pub mod common_shader_generator;

pub mod window;
pub mod lights;

pub mod mesh;
pub mod cube;

pub mod cct;

pub mod rendering_domain;
pub mod world_render_domain;

pub mod visibility_model;
pub mod simple;

pub mod renderer;
pub mod texture_manager;

pub mod render_pass;

pub mod tonemap_render_pass;

pub mod aces_tonemap_render_pass;
pub mod pipeline_manager;

pub mod view;

pub mod csm;

pub mod utils;

/// Maps a shader resource binding to a GHI shader binding descriptor.
pub fn map_shader_binding_to_shader_binding_descriptor(b: &resource_management::spirv_shader_generator::Binding) -> ghi::ShaderBindingDescriptor {
	ghi::ShaderBindingDescriptor::new(b.set, b.binding, if b.read { ghi::AccessPolicies::READ } else { ghi::AccessPolicies::empty() } | if b.write { ghi::AccessPolicies::WRITE } else { ghi::AccessPolicies::empty() })
}

pub fn make_perspective_view_from_camera(camera: &Camera, extent: Extent) -> View {
	let (camera_position, camera_orientation, fov_y) = (camera.get_position(), camera.get_orientation(), camera.get_fov());

	let aspect_ratio = extent.width() as f32 / extent.height() as f32;

	View::new_perspective(fov_y, aspect_ratio, 0.1f32, 100f32, camera_position, camera_orientation)
}
