//! The descriptor set layouts of the visibility pipeline.
//!
//! Each layout is declared by one layer in a module under `assets/rendering/visibility/layouts`, which the pipeline's
//! shaders take as `main` parameters. Each constant here names one layer. The pass that creates a set reflects the
//! layer with [`layout`] and keeps the result, so the set matches every shader baked against that layer, and resources
//! are bound by the names the layer gives them.

use ghi::context::Context as _;

use crate::rendering::set_layout::ENGINE_MODULES;
use crate::rendering::{SetBinder, SetLayout};

/// Names each layer at its `path`, relative to `::byte_engine::rendering::visibility`, and lists every name in `ALL`.
macro_rules! layouts {
	($($(#[$documentation:meta])* $name:ident, $path:literal;)+) => {
		$(
			$(#[$documentation])*
			pub(crate) const $name: &str = concat!("::byte_engine::rendering::visibility::", $path);
		)+
		/// Every layer, so a test can check each one reflects.
		#[cfg(test)]
		const ALL: &[&str] = &[$($name),+];
	};
}

/// Reflects `layer`, one of the constants in this module, from the engine's embedded modules.
///
/// Call it once where the set is created and keep the result: reflection links the layer's module.
pub(crate) fn layout(layer: &str) -> SetLayout {
	SetLayout::from_layer(layer, ENGINE_MODULES)
}

/// Creates one set of `layer` named `name`, and writes the resources `writes` binds to it by BESL name.
///
/// Use it for a set whose resources are all known when it is created; keep the [`layout`] instead when a later frame
/// writes more of them. Next, bind the returned set when recording the pass.
pub(crate) fn create_set<W: AsRef<[ghi::DescriptorWrite]>>(
	context: &mut ghi::implementation::Context,
	layer: &str,
	name: &str,
	writes: impl FnOnce(SetBinder<'_>) -> W,
) -> ghi::DescriptorSetHandle {
	let layout = layout(layer);
	let set = layout.create_set(context, name);
	context.write(writes(layout.binder(set)).as_ref());
	set
}

/// Asserts that each `(layer, buffer, size)` host record is as large as the stride the layer reflects for that buffer,
/// so a record that changes on one side fails a test.
#[cfg(test)]
pub(crate) fn assert_strides(records: &[(&str, &str, usize)]) {
	for &(layer, name, size) in records {
		let stride = layout(layer)
			.interface()
			.binding(name)
			.and_then(|binding| binding.buffer_stride)
			.unwrap_or_else(|| panic!("Expected the layout `{layer}` to declare the buffer `{name}`"));
		assert_eq!(
			stride as usize, size,
			"The host record of `{name}` in `{layer}` has another size than its stride."
		);
	}
}

layouts! {
	/// The scene set every stage that draws or shades the scene binds. The pipeline manager creates it.
	SCENE, "layouts::scene::Scene";
	/// The camera occlusion culling set the task stage reads. See [`super::render_pass::occlusion`].
	OCCLUSION, "layouts::occlusion::Occlusion";
	/// The per-sink set the material dispatch passes count, sort, and list pixels through.
	MATERIAL_DISPATCH, "layouts::material_dispatch::MaterialDispatch";
	/// The per-sink set material evaluation reads the visibility buffer and its pixel lists through.
	VISIBILITY_BUFFER, "layouts::visibility_buffer::VisibilityBuffer";
	/// The per-sink set of material evaluation: the images it shades into and everything lighting reads.
	MATERIAL_EVALUATION, "layouts::material_evaluation::MaterialEvaluation";
	/// The screen view constants at half the sink resolution, which the min depth pyramid, GTAO, and backdrop passes share.
	HALF_RESOLUTION_VIEW, "layouts::half_resolution_view::HalfResolutionView";
	GTAO, "layouts::gtao::Gtao";
	GTAO_BLUR, "layouts::gtao_blur::GtaoBlur";
	GTAO_UPSCALE, "layouts::gtao_upscale::GtaoUpscale";
	/// The per-sink set of the subsurface pass, which spreads subsurface pixels' diffuse light.
	SUBSURFACE, "layouts::subsurface::Subsurface";
	DEPTH_PYRAMID, "layouts::depth_pyramid::DepthPyramid";
	/// The per-sink set of the velocity stage, which writes each pixel's motion since the previous frame.
	VELOCITY, "layouts::velocity::Velocity";
	BACKDROP, "layouts::backdrop::Backdrop";
	/// The per-sink set the screen scene query reads find the instance under each queried pixel through.
	SCREEN_QUERY, "layouts::screen_query::ScreenQuery";
	HIZ_BASE, "layouts::hiz_base::HizBase";
	HIZ_TAIL, "layouts::hiz_tail::HizTail";
	SHADOW_DEPTH_PYRAMID, "layouts::shadow_depth_pyramid::ShadowDepthPyramid";
	/// The set each level past the first of the directional shadow depth pyramids is reduced from the one before through.
	SHADOW_DEPTH_PYRAMID_LEVEL, "layouts::shadow_depth_pyramid::ShadowDepthPyramidLevel";
	/// The set the directional shadow cascade fit rewrites the scene's cascade views through.
	CASCADE_FIT, "layouts::cascade_fit::CascadeFit";
	LIGHT_CLUSTERS, "layouts::light_clusters::LightClusters";
	/// The per-sink set the fog light tile pass and march read the light table, the shadow maps, and the fog through.
	FOG_LIGHT, "layouts::fog_light::FogLight";
	SKINNING, "layouts::skinning::Skinning";
	/// The set the passes that build the frame's mesh work read the instance list and write the work items through.
	/// See [`super::mesh_dispatch::MeshWork`].
	WORK_BUILD, "layouts::work_build::WorkBuild";
	/// The screen view constants and linear depth pyramid a pass that marches screen-space rays reads.
	SCREEN_TRACE, "layouts::screen_trace::ScreenTrace";
	/// The settings the SSGI trace and its temporal filter share.
	SSGI, "layouts::ssgi::Ssgi";
	SSGI_TRACE, "layouts::ssgi_trace::SsgiTrace";
	SSGI_TEMPORAL, "layouts::ssgi_temporal::SsgiTemporal";
	CONTACT_SHADOWS, "layouts::contact_shadows::ContactShadows";
	SUN_VISIBILITY, "layouts::sun_visibility::SunVisibility";
	/// The per-sink set every receiver of a sun's cascades filters them through: the sun visibility resolve for opaque
	/// pixels, and material evaluation for blended surfaces.
	DIRECTIONAL_SHADOWS, "layouts::directional_shadows::DirectionalShadows";
	/// The set the virtual shadow map's page passes and page draw keep the page table and pool through.
	VIRTUAL_SHADOW_PAGES, "layouts::virtual_shadows::VirtualShadowPages";
	/// The set the sun visibility resolve samples the virtual shadow map through.
	VIRTUAL_SHADOW_SAMPLING, "layouts::virtual_shadows::VirtualShadowSampling";
	/// The per-sink set the virtual shadow map's marking reads the sink's receivers through.
	VIRTUAL_SHADOW_MARK, "layouts::virtual_shadows::VirtualShadowMark";
	/// The frame-wide surfel GI set: the surfel pool, its grid, the scene's light table, and the settings every surfel
	/// stage shares.
	SURFEL_GI, "layouts::surfel_gi::SurfelGi";
	/// The per-sink set of the surfel resolve, allocation, and reconstruction: their settings, spawn requests, and the
	/// resolve's images.
	SURFEL_RESOLVE, "layouts::surfel_resolve::SurfelResolve";
	/// The per-sink history the surfel resolve reprojects and the history it accumulates.
	SURFEL_TEMPORAL, "layouts::surfel_temporal::SurfelTemporal";
	/// The per-sink images the surfel spatial pass reads and writes.
	SURFEL_SPATIAL, "layouts::surfel_spatial::SurfelSpatial";
	/// The set the frame's rays trace the scene through: the top-level acceleration structure, the flat triangles, and
	/// the geometry table. See [`super::acceleration_structures`].
	RAY_TRACED_SCENE, "layouts::ray_traced_scene::RayTracedScene";
	/// The hits of the rays of whichever ray-traced technique lights the frame, and the sort that orders them by
	/// material. See [`super::render_pass::RayHits`].
	RAY_HITS, "layouts::ray_hits::RayHits";
	/// The probe volume as its readers see it: the cascades, the probe records and their seen marks, and the atlases
	/// they sample.
	PROBE_VOLUME, "layouts::probe_volume::ProbeVolume";
	/// The frame-wide probe GI set the schedule, trace, shade, and update passes write.
	PROBE_GI, "layouts::probe_gi::ProbeGi";
	/// The per-sink set of the probe resolve: its settings and output images.
	PROBE_RESOLVE, "layouts::probe_resolve::ProbeResolve";

}

#[cfg(test)]
mod tests {
	use std::mem::size_of;

	use super::super::layout::{ShaderMeshLevel, ShaderMeshletData};
	use super::super::render_pass::{FogLightShaderParameters, ScreenViewData};
	use super::super::shader_data::{
		LightClusterParameters, LightingData, MaterialData, ReflectionShaderParameters, ShaderFrame, ShaderMesh, ShaderViewData,
	};
	use super::*;

	#[test]
	fn host_records_match_the_strides_their_layers_reflect() {
		// A table reflects the stride of one record, and a single record reflects its own size.
		assert_strides(&[
			(SCENE, "views", size_of::<ShaderViewData>()),
			(SCENE, "frame", size_of::<ShaderFrame>()),
			(SCENE, "meshes", size_of::<ShaderMesh>()),
			(SCENE, "meshlets", size_of::<ShaderMeshletData>()),
			(SCENE, "materials", size_of::<MaterialData>()),
			(SCENE, "mesh_levels", size_of::<ShaderMeshLevel>()),
			(MATERIAL_EVALUATION, "lighting_data", size_of::<LightingData>()),
			(LIGHT_CLUSTERS, "lighting_data", size_of::<LightingData>()),
			(FOG_LIGHT, "lighting_data", size_of::<LightingData>()),
			(FOG_LIGHT, "parameters", size_of::<FogLightShaderParameters>()),
			(
				MATERIAL_EVALUATION,
				"light_cluster_parameters",
				size_of::<LightClusterParameters>(),
			),
			(
				MATERIAL_EVALUATION,
				"reflection_parameters",
				size_of::<ReflectionShaderParameters>(),
			),
			(MATERIAL_EVALUATION, "half_resolution_view", size_of::<ScreenViewData>()),
			(RAY_TRACED_SCENE, "ray_tracing_geometries", size_of::<[u32; 2]>()),
		]);
	}

	#[test]
	fn every_layer_declares_a_descriptor_set() {
		// Reflection panics when the layer does not link or declares no descriptor.
		for layer in ALL {
			assert!(!layout(layer).interface().bindings.is_empty());
		}
	}
}
