//! The descriptor set layouts of the visibility pipeline.
//!
//! Each layout is declared by one layout layer under `assets/rendering/visibility/layouts`, which the pipeline's
//! shaders list in their sidecars. The layouts here are reflected from the same files, so a set a pass creates from
//! one matches every shader baked against that layer, and resources are bound by the names the layer gives them.

use std::sync::LazyLock;

use crate::rendering::SetLayout;
use crate::rendering::set_layout::layer;

/// Declares the layout of the last of `layers`, which builds on the layers before it.
macro_rules! layout {
	($(#[$documentation:meta])* $name:ident, [$($layer:literal),+ $(,)?]) => {
		$(#[$documentation])*
		pub(crate) static $name: LazyLock<SetLayout> = LazyLock::new(|| SetLayout::from_layers(&[$(layer!($layer)),+]));
	};
}

layout!(
	/// The scene set every stage that draws or shades the scene binds. The pipeline manager creates it.
	SCENE,
	["visibility/layouts/scene-records.besl", "visibility/layouts/scene.besl"]
);
layout!(
	/// The camera occlusion culling set the task stage reads. See [`super::render_pass::occlusion`].
	OCCLUSION,
	["visibility/layouts/occlusion.besl"]
);
layout!(
	/// The per-sink set the material dispatch passes count, sort, and list pixels through.
	MATERIAL_DISPATCH,
	["visibility/layouts/material-dispatch.besl"]
);
layout!(
	/// The per-sink set material evaluation reads the visibility buffer and its pixel lists through.
	VISIBILITY_BUFFER,
	["visibility/layouts/visibility-buffer.besl"]
);
layout!(
	/// The per-sink set of material evaluation: the images it shades into and everything lighting reads.
	MATERIAL_EVALUATION,
	["visibility/layouts/material-evaluation.besl"]
);
layout!(
	/// The screen view constants the depth pyramid and GTAO passes share.
	GTAO_VIEW,
	["visibility/layouts/screen-view.besl", "visibility/layouts/gtao-view.besl"]
);
layout!(GTAO, ["visibility/layouts/gtao.besl"]);
layout!(GTAO_BLUR, ["visibility/layouts/gtao-blur.besl"]);
layout!(GTAO_UPSCALE, ["visibility/layouts/gtao-upscale.besl"]);
layout!(
	DEPTH_PYRAMID,
	[
		"visibility/layouts/receiver-bounds.besl",
		"visibility/layouts/depth-pyramid.besl"
	]
);
layout!(HIZ_BASE, ["visibility/layouts/hiz-base.besl"]);
layout!(HIZ_TAIL, ["visibility/layouts/hiz-tail.besl"]);
layout!(SHADOW_DEPTH_PYRAMID, ["visibility/layouts/shadow-depth-pyramid.besl"]);
layout!(
	/// The set the directional shadow cascade fit rewrites the scene's cascade views through.
	CASCADE_FIT,
	[
		"visibility/layouts/scene-records.besl",
		"visibility/layouts/receiver-bounds.besl",
		"visibility/layouts/cascade-fit.besl"
	]
);
layout!(LIGHT_CLUSTERS, ["visibility/layouts/light-clusters.besl"]);
layout!(
	SKINNING,
	["visibility/layouts/scene-records.besl", "visibility/layouts/skinning.besl"]
);
layout!(
	/// The screen view constants and linear depth pyramid a pass that marches screen-space rays reads.
	SCREEN_TRACE,
	["visibility/layouts/screen-view.besl", "visibility/layouts/screen-trace.besl"]
);
layout!(
	/// The settings the SSGI trace and its temporal filter share.
	SSGI,
	["visibility/layouts/ssgi.besl"]
);
layout!(SSGI_TRACE, ["visibility/layouts/ssgi-trace.besl"]);
layout!(SSGI_TEMPORAL, ["visibility/layouts/ssgi-temporal.besl"]);
layout!(CONTACT_SHADOWS, ["visibility/layouts/contact-shadows.besl"]);
layout!(
	SUN_VISIBILITY,
	[
		"visibility/layouts/screen-view.besl",
		"visibility/layouts/sun-visibility.besl"
	]
);
layout!(
	/// The frame-wide surfel GI set: the surfel pool, its grid, and the settings every surfel stage shares.
	SURFEL_GI,
	["visibility/layouts/surfel-gi.besl"]
);
layout!(
	/// The per-sink set of the surfel resolve and allocation: their settings, spawn requests, and output images.
	SURFEL_RESOLVE,
	["visibility/layouts/surfel-resolve.besl"]
);
layout!(
	/// The set the frame's rays trace the scene through: the top-level acceleration structure, the flat triangles, and
	/// the geometry table. See [`super::acceleration_structures`].
	RAY_TRACED_SCENE,
	["visibility/layouts/ray-traced-scene.besl"]
);

#[cfg(test)]
mod tests {
	use std::mem::size_of;

	use super::super::layout::ShaderMeshletData;
	use super::super::render_pass::ScreenViewData;
	use super::super::shader_data::{
		LightClusterParameters, LightingData, MaterialData, ReflectionShaderParameters, ShaderMesh, ShaderViewData,
	};
	use super::*;

	#[test]
	fn host_records_match_the_strides_their_layout_layers_reflect() {
		let stride = |layout: &SetLayout, name: &str| {
			layout
				.interface()
				.binding(name)
				.and_then(|binding| binding.buffer_stride)
				.unwrap_or_else(|| panic!("Expected the layout to declare the buffer `{name}`")) as usize
		};

		// A table reflects the stride of one record, and a single record reflects its own size.
		assert_eq!(stride(&SCENE, "views"), size_of::<ShaderViewData>());
		assert_eq!(stride(&SCENE, "meshes"), size_of::<ShaderMesh>());
		assert_eq!(stride(&SCENE, "meshlets"), size_of::<ShaderMeshletData>());
		assert_eq!(stride(&SCENE, "materials"), size_of::<MaterialData>());
		assert_eq!(stride(&MATERIAL_EVALUATION, "lighting_data"), size_of::<LightingData>());
		assert_eq!(
			stride(&MATERIAL_EVALUATION, "light_cluster_parameters"),
			size_of::<LightClusterParameters>()
		);
		assert_eq!(
			stride(&MATERIAL_EVALUATION, "reflection_parameters"),
			size_of::<ReflectionShaderParameters>()
		);
		assert_eq!(stride(&MATERIAL_EVALUATION, "ssgi_view"), size_of::<ScreenViewData>());
		assert_eq!(stride(&RAY_TRACED_SCENE, "ray_tracing_geometries"), size_of::<[u32; 2]>());
	}

	#[test]
	fn every_layout_layer_declares_a_descriptor_set() {
		// Each layout reflects its layer on first use, and panics when the layer does not link or declares no descriptor.
		for layout in [
			&SCENE,
			&OCCLUSION,
			&MATERIAL_DISPATCH,
			&VISIBILITY_BUFFER,
			&MATERIAL_EVALUATION,
			&GTAO_VIEW,
			&GTAO,
			&GTAO_BLUR,
			&GTAO_UPSCALE,
			&DEPTH_PYRAMID,
			&HIZ_BASE,
			&HIZ_TAIL,
			&SHADOW_DEPTH_PYRAMID,
			&CASCADE_FIT,
			&LIGHT_CLUSTERS,
			&SKINNING,
			&SCREEN_TRACE,
			&SSGI,
			&SSGI_TRACE,
			&SSGI_TEMPORAL,
			&CONTACT_SHADOWS,
			&SUN_VISIBILITY,
			&SURFEL_GI,
			&SURFEL_RESOLVE,
			&RAY_TRACED_SCENE,
		] {
			assert!(!layout.interface().bindings.is_empty());
		}
	}
}
