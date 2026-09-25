//! Persisted image sequences played at a fixed frame rate.
//!
//! Author a `.flipbook` file that declares the rate and the ordered images:
//!
//! ```json
//! { "frames_per_second": 12, "images": ["char/run0.png", "char/run1.png"] }
//! ```
//!
//! Request the baked [`Flipbook`] by that file's ID, load its [`Flipbook::images`], and sample them with the
//! frame rate, for example through `byte_engine::animation::flipbook::Flipbook`.

use crate::{Reference, ReferenceModel, Solver, resource, resources::image::Image, solver::SolveErrors};

/// The `Flipbook` struct lets a renderer request a whole image sequence and its playback rate by one resource ID.
///
/// The images arrive solved, so the renderer loads each one without requesting it by name.
#[derive(Debug, serde::Serialize)]
pub struct Flipbook {
	/// The number of images shown per second. Baking rejects zero.
	pub frames_per_second: u32,
	/// The images in playback order. Baking rejects an empty sequence.
	pub images: Vec<Reference<Image>>,
}

impl Flipbook {
	/// Takes the images in playback order so each can be loaded.
	pub fn into_images(self) -> Vec<Reference<Image>> {
		self.images
	}
}

/// The `FlipbookModel` struct persists a flipbook with the exact baked images it plays.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct FlipbookModel {
	pub frames_per_second: u32,
	pub images: Vec<ReferenceModel<Image>>,
}
super::impl_resource_model!(Flipbook, FlipbookModel, "Flipbook");

impl<'de> Solver<'de, Reference<Flipbook>> for ReferenceModel<FlipbookModel> {
	/// Restores the flipbook and solves every image it plays.
	fn solve(
		self,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Flipbook>, SolveErrors>> {
		crate::r#async::future(async move {
			let (stored, reader) = storage_backend.read(self.id()).await.ok_or(SolveErrors::StorageError)?;
			let FlipbookModel {
				frames_per_second,
				images: models,
			} = crate::from_slice(&stored.resource).map_err(|error| SolveErrors::DeserializationFailed(error.to_string()))?;

			let mut images = Vec::with_capacity(models.len());
			for image in models {
				images.push(image.solve(storage_backend).await?);
			}

			Ok(Reference::from_model(
				self,
				Flipbook {
					frames_per_second,
					images,
				},
				reader,
			))
		})
	}
}
