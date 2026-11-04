//! Baked lightmaps: the indirect diffuse light of a level's static meshes, held in one image.
//!
//! A `.lightmap.bead` asset names the level to bake. The bake places every primitive with lightmap UVs in a rectangle
//! of the image and records where, so a renderer maps each spawned primitive's lightmap UVs into the image. Request
//! the baked [`Lightmap`] by the asset's ID, then load its [`Lightmap::image`].

use crate::{Reference, ReferenceModel, resource, resources::image::Image, solver::SolveError};

/// The `LightmapInstance` struct records where one primitive of a level's entity lies in its lightmap.
///
/// Renderers find the primitive's entity through [`Lightmap::holder`], then multiply its lightmap UVs by the scale and
/// add the offset to read its texels.
#[derive(
	Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct LightmapInstance {
	/// The entity's spawn index: its position among the level's baked entities.
	pub entity: u32,
	/// The primitive's index in the entity's mesh.
	pub primitive: u32,
	/// Maps the primitive's lightmap UVs into the image: `uv * [x, y] + [z, w]`.
	pub scale_offset: [f32; 4],
}

/// The `Lightmap` struct lets a renderer light a spawned level's static meshes with light baked offline.
///
/// The image arrives solved, so the renderer loads it without requesting it by name.
#[derive(Debug, serde::Serialize)]
pub struct Lightmap {
	/// The spawn index of the level entity that holds the `lightmap` kind. A spawn places entity `n` at the handle
	/// `holder_handle - holder + n`.
	pub holder: u32,
	/// The light: irradiance over π, in nits, as RGBA16F texels.
	pub image: Reference<Image>,
	/// Every primitive with a rectangle, in ascending entity then primitive order.
	pub instances: Vec<LightmapInstance>,
}

/// The `LightmapModel` struct persists a lightmap with the exact baked image it reads.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct LightmapModel {
	pub holder: u32,
	pub image: ReferenceModel<Image>,
	pub instances: Vec<LightmapInstance>,
}
super::impl_resource_model!(Lightmap, LightmapModel, "Lightmap", references = lightmap_references);

/// Lists the image the lightmap reads.
fn lightmap_references(lightmap: &LightmapModel) -> Vec<&str> {
	vec![lightmap.image.id().as_str()]
}

impl crate::StoredModel for LightmapModel {
	type Resource = Lightmap;

	/// Restores the lightmap and solves its image.
	fn solve_stored<'de>(
		stored: crate::SerializableResource,
		reader: crate::resource::resource_handler::MultiResourceReader,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Lightmap>, SolveError>> {
		crate::r#async::future(async move {
			let LightmapModel {
				holder,
				image,
				instances,
			} = crate::from_slice(&stored.resource).map_err(|error| SolveError::DeserializationFailed(error.to_string()))?;
			let image = super::solve_all(vec![image], storage_backend)
				.await?
				.pop()
				.expect("one image solves to one reference");

			Ok(Reference::from_stored(
				stored,
				Lightmap {
					holder,
					image,
					instances,
				},
				reader,
			))
		})
	}
}
