//! Baking `.lightmap.bead` files into lightmaps.

use resource_management::{
	AssetHandler, BakeContext, ProcessedAsset,
	asset::{ResourceId, handler::LoadErrors},
	processors::{ImageProcessorSettings, OwnedImageSource, Semantic, SourceChannels, SourceEncoding},
	resources::{
		image::Image,
		level::Level,
		lightmap::LightmapModel,
		material::{Shader, VariantModel},
		mesh::MeshModel,
		pipeline::Pipeline,
	},
	types::Gamma,
};
use utils::Extent;

use super::bead::LightmapBead;
use super::gpu::{LIGHTMAP_DILATE_PIPELINE, LIGHTMAP_TRACE_SHADER, TraceSettings, trace};
use super::scene::{collect_level, load_level};
use crate::rendering::pipelines::visibility::{geometry::GENERATED_MESH_MATERIAL, ray_tracing::DEFAULT_HIT_SHADER};
use crate::rendering::renderable::mesh::MeshSource;

const DOCS_PATH: &str = "reference/assets#lightmaps";
const LIGHTMAP_UVS_DOCS_PATH: &str = "reference/assets#lightmap-uvs";

/// The `LightmapAssetHandler` struct lets an asset manager bake `.lightmap.bead` files into the lightmaps that light a
/// level's static meshes when `render.gi.technique` is `baked`.
///
/// Baking a lightmap bakes its level and everything the level draws first, then path traces the light on the GPU with
/// the same meshes, materials, and closest-hit shaders the renderer uses, and stores one
/// [`Lightmap`](resource_management::resources::lightmap::Lightmap) under the file's ID. It needs a GPU that traces
/// rays. [`configure_default_baking`](crate::application::graphics::defaults::configure_default_baking) registers it
/// when `render.baked-gi.enabled` is set; next, name the lightmap in the level with the `lightmap` kind.
#[derive(Clone, Copy, Debug, Default)]
pub struct LightmapAssetHandler;

impl AssetHandler for LightmapAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("lightmap.bead")
	}

	/// Parses the file, bakes what it lights, traces its light on the GPU, and stores the image and the lightmap.
	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (source, asset_type) = context.resolve(id).await?;
		if !self.can_handle(&asset_type) {
			return Err(LoadErrors::UnsupportedType);
		}
		let invalid = |cause: &str| {
			context.error(format_args!(
				"Lightmap '{}' is invalid. The most likely cause is {cause}. Fix the file or its level as the format specifies; see {}.",
				id.as_ref(),
				utils::online_docs_url(DOCS_PATH)
			));
			LoadErrors::FailedToProcess
		};
		let bead = LightmapBead::parse(&source).map_err(|cause| invalid(&cause))?;

		// The level and everything it draws bake first, so the trace loads the resources the renderer loads.
		context.bake_dependency::<Level>(&bead.level).await?;
		let (level, payload) = load_level(&context.resource_manager(), &bead.level)
			.await
			.map_err(|cause| invalid(&cause))?;
		let scene = collect_level(id.as_ref(), &level, &payload).map_err(|cause| invalid(&cause))?;
		let mut meshes = Vec::new();
		let mut materials = Vec::new();
		for mesh in &scene.meshes {
			match &mesh.source {
				MeshSource::Resource(mesh) if !meshes.iter().any(|known: &String| known == mesh.as_ref()) => {
					meshes.push(mesh.to_string());
				}
				MeshSource::Resource(_) => {}
				MeshSource::Generated(generator) => {
					let material = generator.material().unwrap_or(GENERATED_MESH_MATERIAL).to_string();
					if !materials.contains(&material) {
						materials.push(material);
					}
				}
			}
		}
		context.bake_dependencies::<MeshModel>(&meshes, 8).await?;
		context.bake_dependencies::<VariantModel>(&materials, 8).await?;
		if let Some(environment) = &bead.environment {
			context.bake_dependency::<Image>(environment).await?;
		}
		context.bake_dependency::<Shader>(LIGHTMAP_TRACE_SHADER).await?;
		context.bake_dependency::<Shader>(DEFAULT_HIT_SHADER).await?;
		context.bake_dependency::<Pipeline>(LIGHTMAP_DILATE_PIPELINE).await?;

		let holder = scene.holder;
		let settings = TraceSettings {
			samples: bead.samples,
			bounces: bead.bounces,
			texel_density: bead.texel_density,
			max_extent: bead.max_extent,
			environment: bead.environment.clone(),
			illuminance: bead.illuminance,
		};
		context.info(format_args!(
			"Tracing lightmap '{}' with {} paths per texel.",
			id.as_ref(),
			settings.samples
		));
		// The trace blocks on the GPU, so it runs on a thread of its own while this bake waits.
		let (sender, receiver) = utils::r#async::oneshot::channel();
		let (resources, shaders) = (context.resource_manager(), context.resource_manager());
		std::thread::Builder::new()
			.name("Lightmap Bake".into())
			.spawn(move || {
				let _ = sender.send(trace(resources, shaders, scene, settings));
			})
			.map_err(|_| invalid("that the bake couldn't start its GPU thread"))?;
		let traced = receiver
			.await
			.map_err(|_| invalid("that the bake's GPU thread stopped unexpectedly"))?
			.map_err(|cause| {
				context.error(format_args!(
					"Lightmap '{}' could not be traced. The most likely cause is {cause}. Bake it on a machine whose GPU traces rays, or check the level's resources; see {}.",
					id.as_ref(),
					utils::online_docs_url(DOCS_PATH)
				));
				LoadErrors::FailedToProcess
			})?;

		for (entity, primitive) in &traced.unlit {
			context.warn(format_args!(
				"Lightmap '{}' leaves primitive {primitive} of entity {entity} unlit. The most likely cause is a mesh baked without lightmap UVs. Bake the mesh with `lightmap_uvs` on; see {}.",
				id.as_ref(),
				utils::online_docs_url(LIGHTMAP_UVS_DOCS_PATH)
			));
		}
		if let Some(factor) = traced.shrunk {
			context.warn(format_args!(
				"Lightmap '{}' holds {:.2} texels per meter instead of {}. The most likely cause is that the level's surfaces don't fit a {} × {} image at the requested density. Raise `max_extent` or lower `texel_density`.",
				id.as_ref(),
				bead.texel_density * factor,
				bead.texel_density,
				bead.max_extent,
				bead.max_extent
			));
		}

		// The light is stored as half floats, without mips: the renderer reads it texel for texel. A texel without light
		// stores no view of the environment either.
		let mut data = Vec::with_capacity_in(traced.light.len() * 8, context.allocator());
		for &[red, green, blue, visibility] in &traced.light {
			for channel in [red, green, blue, visibility.max(0.0)] {
				data.extend_from_slice(&half::f16::from_f32(channel).to_le_bytes());
			}
		}
		let image = context
			.image_processor()
			.process_owned_in(
				Semantic::Other,
				Gamma::Linear,
				ImageProcessorSettings::LOOKUP,
				|| {
					Ok(OwnedImageSource {
						extent: Extent::square(traced.extent),
						channels: SourceChannels::RGBA,
						encoding: SourceEncoding::F16LittleEndian,
						data,
					})
				},
				context.allocator(),
			)
			.await?;
		let image_id = format!("{}#image", id.as_ref());
		let (asset, data) = image.into_asset(ResourceId::new(&image_id));
		let image = context.store_resource(asset, &data).await?;
		let lightmap = LightmapModel {
			holder,
			image: image.into(),
			instances: traced.instances,
		};
		context.store_primary(ProcessedAsset::new(id, lightmap), &[]).await
	}
}
