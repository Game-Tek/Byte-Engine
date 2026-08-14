use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
	CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
};
use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::implementation::Frame;
use utils::{Box, Extent, RGBA};

use crate::configuration::ConfigurationValue;
use crate::rendering::pipelines::visibility::mesh_dispatch::MeshDispatch;
use crate::rendering::pipelines::visibility::pipeline_manager::Instance;
use crate::rendering::pipelines::visibility::skinning::{SkinningDispatch, SkinningPass};
use crate::rendering::pipelines::visibility::{
	ActiveMaterialMask, CONE_SHADOW_MAP_RESOLUTION, CONE_SHADOW_VIEW_OFFSET, INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING,
	MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING,
	MAX_CONE_SHADOW_POOL_CAPACITY, MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIALS, MAX_MESHLETS, MAX_POINT_SHADOW_POOL_CAPACITY,
	MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES, MESHLET_DATA_BINDING, MESH_DATA_BINDING, POINT_SHADOW_FACE_COUNT,
	POINT_SHADOW_MAP_RESOLUTION, POINT_SHADOW_VIEW_OFFSET, PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT,
	SHADOW_MAP_RESOLUTION, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING,
	VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{render_pass::RenderPassReturn, RenderPass, Sink};

mod gtao;
mod materials;
mod orchestration;
mod shadows;
mod visibility;

pub use gtao::*;
#[cfg(test)]
pub(crate) use gtao::{fast_gtao_view_data, gtao_half_resolution_extent};
pub use materials::*;
pub use orchestration::*;
pub use shadows::*;
#[cfg(test)]
pub(crate) use shadows::{cone_shadow_view_indices, directional_shadow_view_indices, point_shadow_view_indices};
pub use visibility::*;

#[cfg(test)]
mod tests {
	use math::{inverse, Point, UnitVector};
	use maths_rs::{cross, dot, length, Vec3f, Vec4f};
	use utils::Extent;

	use super::{
		cone_shadow_view_indices, directional_shadow_view_indices, fast_gtao_view_data, gtao_half_resolution_extent,
		point_shadow_view_indices, transparent_visibility_layer, GtaoSettings, Instance, MeshDispatch,
	};
	use crate::configuration::ConfigurationValue;
	use crate::rendering::pipelines::visibility::{
		CONE_SHADOW_VIEW_OFFSET, MAX_CONE_SHADOW_POOL_CAPACITY, MAX_POINT_SHADOW_POOL_CAPACITY, POINT_SHADOW_FACE_COUNT,
		POINT_SHADOW_VIEW_OFFSET,
	};
	use crate::rendering::{view::View, Sink};

	#[test]
	fn shadow_dispatches_preserve_directional_cascades_cone_layers_and_point_cube_faces() {
		let dispatch = MeshDispatch::with_workgroup_count(19);

		assert_eq!(directional_shadow_view_indices(dispatch).collect::<Vec<_>>(), [1, 2, 3, 4]);
		assert_eq!(
			cone_shadow_view_indices(dispatch, 4).collect::<Vec<_>>(),
			[(5, 0), (6, 1), (7, 2), (8, 3)]
		);
		assert_eq!(
			cone_shadow_view_indices(dispatch, MAX_CONE_SHADOW_POOL_CAPACITY + 1).last(),
			Some((
				(CONE_SHADOW_VIEW_OFFSET + MAX_CONE_SHADOW_POOL_CAPACITY - 1) as u32,
				(MAX_CONE_SHADOW_POOL_CAPACITY - 1) as u32
			))
		);
		assert_eq!(directional_shadow_view_indices(MeshDispatch::default()).count(), 0);
		assert_eq!(cone_shadow_view_indices(MeshDispatch::default(), 4).count(), 0);
		assert_eq!(
			point_shadow_view_indices(dispatch, 2).collect::<Vec<_>>(),
			[
				(POINT_SHADOW_VIEW_OFFSET as u32, 0),
				((POINT_SHADOW_VIEW_OFFSET + 1) as u32, 1),
				((POINT_SHADOW_VIEW_OFFSET + 2) as u32, 2),
				((POINT_SHADOW_VIEW_OFFSET + 3) as u32, 3),
				((POINT_SHADOW_VIEW_OFFSET + 4) as u32, 4),
				((POINT_SHADOW_VIEW_OFFSET + 5) as u32, 5),
				((POINT_SHADOW_VIEW_OFFSET + 6) as u32, 6),
				((POINT_SHADOW_VIEW_OFFSET + 7) as u32, 7),
				((POINT_SHADOW_VIEW_OFFSET + 8) as u32, 8),
				((POINT_SHADOW_VIEW_OFFSET + 9) as u32, 9),
				((POINT_SHADOW_VIEW_OFFSET + 10) as u32, 10),
				((POINT_SHADOW_VIEW_OFFSET + 11) as u32, 11),
			]
		);
		assert_eq!(
			point_shadow_view_indices(dispatch, MAX_POINT_SHADOW_POOL_CAPACITY + 1).last(),
			Some((
				(POINT_SHADOW_VIEW_OFFSET + MAX_POINT_SHADOW_POOL_CAPACITY * POINT_SHADOW_FACE_COUNT - 1) as u32,
				(MAX_POINT_SHADOW_POOL_CAPACITY * POINT_SHADOW_FACE_COUNT - 1) as u32,
			))
		);
		assert_eq!(point_shadow_view_indices(MeshDispatch::default(), 4).count(), 0);
	}

	#[test]
	fn gtao_runtime_parameters_update_quality_controls_without_partial_state() {
		let defaults = GtaoSettings::default();
		let (settings, radius) = defaults
			.with_parameter("radius", &ConfigurationValue::Text("2.5".to_string()))
			.expect("radius should parse");
		let (settings, samples) = settings
			.with_parameter("samples-per-ray", &ConfigurationValue::Integer(12))
			.expect("sample count should parse");
		let (settings, rays) = settings
			.with_parameter("radial-rays", &ConfigurationValue::Integer(16))
			.expect("ray count should parse");

		assert_eq!(settings.radius, 2.5);
		assert_eq!(settings.samples_per_ray, 12);
		assert_eq!(settings.radial_rays, 16);
		assert_eq!(radius, ConfigurationValue::Float(2.5));
		assert_eq!(samples, ConfigurationValue::Integer(12));
		assert_eq!(rays, ConfigurationValue::Integer(16));

		assert!(settings
			.with_parameter("radial-rays", &ConfigurationValue::Integer(7))
			.is_err());
		assert_eq!(settings.radial_rays, 16);
	}

	#[test]
	fn transparent_visibility_uses_one_depth_resolved_layer() {
		let instances = [
			Instance {
				shader_mesh_index: 3,
				meshlet_count: 2,
			},
			Instance {
				shader_mesh_index: 8,
				meshlet_count: 5,
			},
		];

		let layer = transparent_visibility_layer(&instances).expect("Non-empty transparent work must produce one layer");

		assert_eq!(layer, instances);
		assert!(transparent_visibility_layer(&[]).is_none());
		assert!(transparent_visibility_layer(&[Instance {
			shader_mesh_index: 13,
			meshlet_count: 0,
		}])
		.is_none());
	}

	#[test]
	fn fast_gtao_view_reconstructs_pixel_rays_and_reversed_depth() {
		let extent = Extent::rectangle(1920, 1080);
		let view = View::new_perspective(
			60.0,
			extent.width() as f32 / extent.height() as f32,
			0.1,
			100.0,
			Point::origin(),
			UnitVector::z_axis(),
		);
		let sink = Sink::new(view, extent, 0);
		let gtao_extent = gtao_half_resolution_extent(extent);
		let constants = fast_gtao_view_data(&sink, gtao_extent);
		let projection = view.projection();
		assert_eq!(std::mem::size_of_val(&constants), 32);

		for z in [0.1f32, 0.5, 1.0, 10.0, 100.0] {
			let clip = projection * Vec4f::new(0.0, 0.0, z, 1.0);
			let depth = clip.z / clip.w;
			let reconstructed = constants.depth_unproject_numerator / (depth + constants.depth_unproject_denominator_offset);
			assert!(
				(reconstructed - z).abs() <= z.max(1.0) * 0.00001,
				"Unexpected GTAO depth reconstruction for z={z}: {reconstructed}"
			);
		}

		for pixel in [[0.0f32, 0.0], [479.0, 269.0], [959.0, 539.0]] {
			let ray = [
				pixel[0] * constants.pixel_to_ray_mul[0] + constants.pixel_to_ray_add[0],
				pixel[1] * constants.pixel_to_ray_mul[1] + constants.pixel_to_ray_add[1],
			];
			let ndc = [
				2.0 * (pixel[0] + 0.5) / gtao_extent.width() as f32 - 1.0,
				1.0 - 2.0 * (pixel[1] + 0.5) / gtao_extent.height() as f32,
			];
			assert!((ray[0] - ndc[0] / projection[0]).abs() < 0.000001);
			assert!((ray[1] - ndc[1] / projection[5]).abs() < 0.000001);
		}
		assert_eq!(constants.view_z_sign, 1.0);
		assert_eq!(gtao_extent, Extent::rectangle(960, 540));
		assert_eq!(
			gtao_half_resolution_extent(Extent::rectangle(1919, 1079)),
			Extent::rectangle(959, 539)
		);
		assert_eq!(gtao_half_resolution_extent(Extent::square(1)), Extent::square(1));
	}

	#[test]
	fn gtao_view_space_reconstruction_z_is_positive() {
		let near = 0.1f32;
		let far = 100.0f32;
		let fov = 45.0f32;
		let aspect = 16.0 / 9.0;
		let extent_x = 1920i32;
		let extent_y = 1080i32;

		let proj = math::projection_matrix(fov, aspect, near, far);
		let inv_proj = inverse(proj);

		// Simulate what the GTAO shader does: reconstruct positions for center + neighbors
		// at various depths, compute the normal, and check its direction

		let reconstruct = |px: i32, py: i32, depth: f32| -> Vec3f {
			let uv_x = (px as f32 + 0.5) / extent_x as f32;
			let uv_y = (py as f32 + 0.5) / extent_y as f32;
			let ndc_x = uv_x * 2.0 - 1.0;
			let ndc_y = 1.0 - uv_y * 2.0;
			let clip = Vec4f::new(ndc_x, ndc_y, depth, 1.0);
			let view = inv_proj * clip;
			let w = view.w;
			Vec3f::new(view.x / w, view.y / w, view.z / w)
		};

		// Project a known view-space point to get its depth
		let project_to_depth = |vx: f32, vy: f32, vz: f32| -> f32 {
			let clip = proj * Vec4f::new(vx, vy, vz, 1.0);
			clip.z / clip.w // ndc depth
		};

		// Test at different distances
		for vz in [0.5f32, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0] {
			let depth = project_to_depth(0.0, 0.0, vz);
			let center_px = extent_x / 2;
			let center_py = extent_y / 2;

			let center = reconstruct(center_px, center_py, depth);
			let right = reconstruct(center_px + 1, center_py, depth);
			let left = reconstruct(center_px - 1, center_py, depth);
			let top = reconstruct(center_px, center_py - 1, depth);
			let bottom = reconstruct(center_px, center_py + 1, depth);

			// min_diff for horizontal: pick shorter of (right - center) or (center - left)
			let ap_h = Vec3f::new(right.x - center.x, right.y - center.y, right.z - center.z);
			let bp_h = Vec3f::new(center.x - left.x, center.y - left.y, center.z - left.z);
			let h_diff = if dot(ap_h, ap_h) < dot(bp_h, bp_h) { ap_h } else { bp_h };

			// min_diff for vertical: pick shorter of (top - center) or (center - bottom)
			let ap_v = Vec3f::new(top.x - center.x, top.y - center.y, top.z - center.z);
			let bp_v = Vec3f::new(center.x - bottom.x, center.y - bottom.y, center.z - bottom.z);
			let v_diff = if dot(ap_v, ap_v) < dot(bp_v, bp_v) { ap_v } else { bp_v };

			let normal = cross(h_diff, v_diff);
			let normal_len = length(normal);
			let normal = if normal_len > 1e-8 {
				Vec3f::new(normal.x / normal_len, normal.y / normal_len, normal.z / normal_len)
			} else {
				Vec3f::new(0.0, 0.0, 1.0)
			};

			// The shader enforces camera-facing: if dot(normal, center_position) > 0, flip.
			// In view space the camera is at origin, so center_position IS the view direction to the point.
			let dot_n_p = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			let normal = if dot_n_p > 0.0 {
				Vec3f::new(-normal.x, -normal.y, -normal.z)
			} else {
				normal
			};

			eprintln!(
				"vz={:.1}: center=({:.4},{:.4},{:.4}), normal=({:.4},{:.4},{:.4}), depth={:.6}",
				vz, center.x, center.y, center.z, normal.x, normal.y, normal.z, depth
			);

			// The normal must face toward the camera, i.e. dot(normal, center_position) <= 0.
			// For a flat surface perpendicular to Z: normal.z should be dominant and negative.
			let dot_check = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			assert!(
				dot_check <= 0.0,
				"Normal should face camera (dot(normal, center_position) <= 0) at vz={}, got dot={}",
				vz,
				dot_check
			);
			assert!(
				normal.z.abs() > 0.99,
				"Normal Z should be dominant for flat surface perpendicular to Z at vz={}, got normal.z={}",
				vz,
				normal.z
			);
		}
	}

	/// Simulates the GTAO normal reconstruction on a floor plane (Y=constant)
	/// where depth varies per pixel, and checks for normal sign flips at different distances.
	#[test]
	fn gtao_normal_on_floor_plane() {
		let near = 0.1f32;
		let far = 100.0f32;
		let fov = 45.0f32;
		let aspect = 16.0 / 9.0;
		let extent_x = 1920i32;
		let extent_y = 1080i32;

		let proj = math::projection_matrix(fov, aspect, near, far);
		let inv_proj = inverse(proj);

		let reconstruct = |px: i32, py: i32, depth: f32| -> Vec3f {
			let uv_x = (px as f32 + 0.5) / extent_x as f32;
			let uv_y = (py as f32 + 0.5) / extent_y as f32;
			let ndc_x = uv_x * 2.0 - 1.0;
			let ndc_y = 1.0 - uv_y * 2.0;
			let clip = Vec4f::new(ndc_x, ndc_y, depth, 1.0);
			let view = inv_proj * clip;
			Vec3f::new(view.x / view.w, view.y / view.w, view.z / view.w)
		};

		let project = |vx: f32, vy: f32, vz: f32| -> (f32, f32, f32) {
			let clip = proj * Vec4f::new(vx, vy, vz, 1.0);
			let ndc_x = clip.x / clip.w;
			let ndc_y = clip.y / clip.w;
			let depth = clip.z / clip.w;
			// Inverse of: ndc_x = uv_x * 2 - 1, ndc_y = 1 - uv_y * 2
			let uv_x = (ndc_x + 1.0) / 2.0;
			let uv_y = (1.0 - ndc_y) / 2.0;
			let px = uv_x * extent_x as f32 - 0.5;
			let py = uv_y * extent_y as f32 - 0.5;
			(px, py, depth)
		};

		// Floor plane at Y = -1 (camera looks along +Z, floor is below camera)
		// For a given pixel, we need to find where the ray through that pixel hits Y=-1
		let floor_y = -1.0f32;

		// For a pixel (px, py), reconstruct a ray direction in view space:
		// The ray goes from origin through the point at depth=1 (arbitrary)
		let ray_hit_floor = |px: i32, py: i32| -> Option<(f32, f32)> {
			// Reconstruct view-space direction using depth=0.5 (arbitrary non-zero)
			let p = reconstruct(px, py, 0.5);
			// Ray: origin=(0,0,0), direction=p (normalized doesn't matter, just need ratio)
			// Hit Y=floor_y: t = floor_y / p.y
			if p.y.abs() < 1e-8 {
				return None;
			} // ray parallel to floor
			let t = floor_y / p.y;
			if t <= 0.0 {
				return None;
			} // floor behind camera
			let hit_z = p.z * t;
			if hit_z < near || hit_z > far {
				return None;
			} // outside clip range
	 // Project hit point to get depth
			let hit_x = p.x * t;
			let clip = proj * Vec4f::new(hit_x, floor_y, hit_z, 1.0);
			Some((hit_z, clip.z / clip.w))
		};

		let min_diff = |p: Vec3f, a: Vec3f, b: Vec3f| -> Vec3f {
			let ap = Vec3f::new(a.x - p.x, a.y - p.y, a.z - p.z);
			let bp = Vec3f::new(p.x - b.x, p.y - b.y, p.z - b.z);
			if dot(ap, ap) < dot(bp, bp) {
				ap
			} else {
				bp
			}
		};

		eprintln!("\n--- Floor plane normal reconstruction ---");
		eprintln!("Testing at various screen Y positions (floor at Y={}):", floor_y);

		let mut found_flip = false;

		// Test across different screen rows (different distances to floor)
		for py in (extent_y / 2 + 50..extent_y - 10).step_by(50) {
			let px = extent_x / 2; // screen center X

			let Some((center_vz, center_depth)) = ray_hit_floor(px, py) else {
				continue;
			};
			let Some((_, left_depth)) = ray_hit_floor(px - 1, py) else {
				continue;
			};
			let Some((_, right_depth)) = ray_hit_floor(px + 1, py) else {
				continue;
			};
			let Some((_, top_depth)) = ray_hit_floor(px, py - 1) else {
				continue;
			};
			let Some((_, bottom_depth)) = ray_hit_floor(px, py + 1) else {
				continue;
			};

			let center = reconstruct(px, py, center_depth);
			let left = reconstruct(px - 1, py, left_depth);
			let right = reconstruct(px + 1, py, right_depth);
			let top = reconstruct(px, py - 1, top_depth);
			let bottom = reconstruct(px, py + 1, bottom_depth);

			let h_diff = min_diff(center, right, left);
			let v_diff = min_diff(center, top, bottom);

			let normal = cross(h_diff, v_diff);
			let normal_len = length(normal);
			let normal = if normal_len > 1e-8 {
				Vec3f::new(normal.x / normal_len, normal.y / normal_len, normal.z / normal_len)
			} else {
				Vec3f::new(0.0, 0.0, 1.0)
			};

			// Apply camera-facing check (same as shader)
			let dot_n_p = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			let normal = if dot_n_p > 0.0 {
				Vec3f::new(-normal.x, -normal.y, -normal.z)
			} else {
				normal
			};

			eprintln!(
				"py={:4}, vz={:6.2}: h_diff=({:+.6},{:+.6},{:+.6}), v_diff=({:+.6},{:+.6},{:+.6}), normal=({:+.4},{:+.4},{:+.4})",
				py, center_vz, h_diff.x, h_diff.y, h_diff.z, v_diff.x, v_diff.y, v_diff.z, normal.x, normal.y, normal.z,
			);

			// For a floor plane at Y=-1, the normal should point +Y (up, toward camera if cam is above floor)
			if normal.y < 0.0 {
				found_flip = true;
				eprintln!("  ^^^ FLIPPED! Normal Y is negative (pointing into floor)");
			}
		}

		if found_flip {
			eprintln!("\nWARNING: Normal flipped at some distances! This explains the hard boundary.");
		} else {
			eprintln!("\nAll normals consistent (no flip detected in tested range).");
		}
	}
}
