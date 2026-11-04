//! The descriptor set layouts of the visibility pipeline.
//!
//! Each layout is declared by one layer in a module under `assets/rendering/visibility/layouts`, which the pipeline's
//! shaders take as `main` parameters. The layouts here are reflected from the same modules, so a set a pass creates
//! from one matches every shader baked against that layer, and resources are bound by the names the layer gives them.

use std::sync::LazyLock;

use crate::rendering::SetLayout;
use crate::rendering::set_layout::ENGINE_MODULES;

/// Declares the layout of the layer at `path`, relative to `::byte_engine::rendering::visibility`.
macro_rules! layout {
	($(#[$documentation:meta])* $name:ident, $path:literal) => {
		$(#[$documentation])*
		pub(crate) static $name: LazyLock<SetLayout> = LazyLock::new(|| {
			SetLayout::from_layer(concat!("::byte_engine::rendering::visibility::", $path), ENGINE_MODULES)
		});
	};
}

layout!(
	/// The scene set every stage that draws or shades the scene binds. The pipeline manager creates it.
	SCENE,
	"layouts::scene::Scene"
);
layout!(
	/// The camera occlusion culling set the task stage reads. See [`super::render_pass::occlusion`].
	OCCLUSION,
	"layouts::occlusion::Occlusion"
);
layout!(
	/// The per-sink set the material dispatch passes count, sort, and list pixels through.
	MATERIAL_DISPATCH,
	"layouts::material_dispatch::MaterialDispatch"
);
layout!(
	/// The per-sink set material evaluation reads the visibility buffer and its pixel lists through.
	VISIBILITY_BUFFER,
	"layouts::visibility_buffer::VisibilityBuffer"
);
layout!(
	/// The per-sink set of material evaluation: the images it shades into and everything lighting reads.
	MATERIAL_EVALUATION,
	"layouts::material_evaluation::MaterialEvaluation"
);
layout!(
	/// The screen view constants the depth pyramid and GTAO passes share.
	GTAO_VIEW,
	"layouts::gtao_view::GtaoView"
);
layout!(GTAO, "layouts::gtao::Gtao");
layout!(GTAO_BLUR, "layouts::gtao_blur::GtaoBlur");
layout!(GTAO_UPSCALE, "layouts::gtao_upscale::GtaoUpscale");
layout!(DEPTH_PYRAMID, "layouts::depth_pyramid::DepthPyramid");
layout!(HIZ_BASE, "layouts::hiz_base::HizBase");
layout!(HIZ_TAIL, "layouts::hiz_tail::HizTail");
layout!(SHADOW_DEPTH_PYRAMID, "layouts::shadow_depth_pyramid::ShadowDepthPyramid");
layout!(
	/// The set the directional shadow cascade fit rewrites the scene's cascade views through.
	CASCADE_FIT,
	"layouts::cascade_fit::CascadeFit"
);
layout!(LIGHT_CLUSTERS, "layouts::light_clusters::LightClusters");
layout!(
	/// The per-sink set the fog light tile pass and march read the light table, the shadow maps, and the fog through.
	FOG_LIGHT,
	"layouts::fog_light::FogLight"
);
layout!(SKINNING, "layouts::skinning::Skinning");
layout!(
	/// The screen view constants and linear depth pyramid a pass that marches screen-space rays reads.
	SCREEN_TRACE,
	"layouts::screen_trace::ScreenTrace"
);
layout!(
	/// The settings the SSGI trace and its temporal filter share.
	SSGI,
	"layouts::ssgi::Ssgi"
);
layout!(SSGI_TRACE, "layouts::ssgi_trace::SsgiTrace");
layout!(SSGI_TEMPORAL, "layouts::ssgi_temporal::SsgiTemporal");
layout!(CONTACT_SHADOWS, "layouts::contact_shadows::ContactShadows");
layout!(SUN_VISIBILITY, "layouts::sun_visibility::SunVisibility");
layout!(
	/// The frame-wide surfel GI set: the surfel pool, its grid, the scene's light table, and the settings every surfel
	/// stage shares.
	SURFEL_GI,
	"layouts::surfel_gi::SurfelGi"
);
layout!(
	/// The per-sink set of the surfel resolve, allocation, and reconstruction: their settings, spawn requests, and the
	/// resolve's images.
	SURFEL_RESOLVE,
	"layouts::surfel_resolve::SurfelResolve"
);
layout!(
	/// The per-sink history the surfel resolve reprojects and the history it accumulates.
	SURFEL_TEMPORAL,
	"layouts::surfel_temporal::SurfelTemporal"
);
layout!(
	/// The per-sink images the surfel spatial pass reads and writes.
	SURFEL_SPATIAL,
	"layouts::surfel_spatial::SurfelSpatial"
);
layout!(
	/// The set the frame's rays trace the scene through: the top-level acceleration structure, the flat triangles, and
	/// the geometry table. See [`super::acceleration_structures`].
	RAY_TRACED_SCENE,
	"layouts::ray_traced_scene::RayTracedScene"
);
layout!(
	/// The hits of the rays of whichever ray-traced technique lights the frame, and the sort that orders them by
	/// material. See [`super::render_pass::RayHits`].
	RAY_HITS,
	"layouts::ray_hits::RayHits"
);
layout!(
	/// The probe volume as its readers see it: the cascades, the probe records and their seen marks, and the atlases
	/// they sample.
	PROBE_VOLUME,
	"layouts::probe_volume::ProbeVolume"
);
layout!(
	/// The frame-wide probe GI set the schedule, trace, shade, and update passes write.
	PROBE_GI,
	"layouts::probe_gi::ProbeGi"
);
layout!(
	/// The per-sink set of the probe resolve: its settings and output images.
	PROBE_RESOLVE,
	"layouts::probe_resolve::ProbeResolve"
);

#[cfg(test)]
mod tests {
	use std::mem::size_of;

	use super::super::layout::ShaderMeshletData;
	use super::super::render_pass::{FogLightShaderParameters, ScreenViewData};
	use super::super::shader_data::{
		LightClusterParameters, LightingData, MaterialData, ReflectionShaderParameters, ShaderMesh, ShaderViewData,
	};
	use super::*;

	#[test]
	fn host_records_match_the_strides_their_layers_reflect() {
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
		assert_eq!(
			stride(&SCENE, "instance_lightmaps"),
			size_of::<super::super::shader_data::InstanceLightmap>()
		);
		assert_eq!(stride(&MATERIAL_EVALUATION, "lighting_data"), size_of::<LightingData>());
		assert_eq!(stride(&LIGHT_CLUSTERS, "lighting_data"), size_of::<LightingData>());
		assert_eq!(stride(&FOG_LIGHT, "lighting_data"), size_of::<LightingData>());
		assert_eq!(stride(&FOG_LIGHT, "parameters"), size_of::<FogLightShaderParameters>());
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
	fn every_layer_declares_a_descriptor_set() {
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
			&FOG_LIGHT,
			&SKINNING,
			&SCREEN_TRACE,
			&SSGI,
			&SSGI_TRACE,
			&SSGI_TEMPORAL,
			&CONTACT_SHADOWS,
			&SUN_VISIBILITY,
			&SURFEL_GI,
			&SURFEL_RESOLVE,
			&SURFEL_TEMPORAL,
			&SURFEL_SPATIAL,
			&RAY_TRACED_SCENE,
			&RAY_HITS,
			&PROBE_VOLUME,
			&PROBE_GI,
			&PROBE_RESOLVE,
		] {
			assert!(!layout.interface().bindings.is_empty());
		}
	}
}
