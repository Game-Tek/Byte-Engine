// Resource declarations repeat the same persisted class and runtime-model association.
// Keeping that contract in one explicit invocation prevents the two identifiers from drifting.
macro_rules! impl_resource_model {
	($resource:ty, $model:ty, $class:literal) => {
		impl $crate::Resource for $resource {
			type Model = $model;
		}

		impl $crate::Model for $model {
			fn get_class() -> &'static str {
				$class
			}
		}
	};
}

pub(crate) use impl_resource_model;

// Direct resources use the same type for persisted metadata and runtime access.
// This remains opt-in so resources that resolve dependencies can keep specialized solvers.
macro_rules! impl_direct_resource {
	($resource:ty, $class:literal) => {
		$crate::resources::impl_resource_model!($resource, $resource, $class);

		impl $crate::StoredModel for $resource {
			type Resource = $resource;

			/// Restores direct resource metadata while retaining its binary-data reader.
			fn solve_stored<'de>(
				stored: $crate::SerializableResource,
				reader: $crate::resource::resource_handler::MultiResourceReader,
				_: &'de dyn $crate::resource::DynReadStorageBackend,
			) -> $crate::r#async::BoxedFuture<'de, Result<$crate::Reference<$resource>, $crate::solver::SolveErrors>> {
				$crate::r#async::future(async move {
					let resource: $resource = $crate::from_slice(stored.resource())
						.map_err(|error| $crate::solver::SolveErrors::DeserializationFailed(error.to_string()))?;

					Ok($crate::Reference::from_stored(stored, resource, reader))
				})
			}
		}
	};
}

pub(crate) use impl_direct_resource;

/// Bounds how many independent dependencies of one resource are read from storage at once.
const DEPENDENCY_SOLVE_CONCURRENCY: usize = 8;

/// Solves independent dependencies concurrently and returns them in input order.
///
/// Solvers call this for lists such as a material's shaders or a variant's variables, which do not depend on
/// one another, so their storage reads overlap instead of running one after another.
pub(crate) async fn solve_all<'de, M, T>(
	models: Vec<M>,
	storage_backend: &'de dyn crate::resource::DynReadStorageBackend,
) -> Result<Vec<T>, crate::solver::SolveErrors>
where
	M: crate::Solver<'de, T> + 'de,
{
	use utils::r#async::stream::{self, TryStreamExt as _};

	// The first failure ends the solve instead of waiting for the remaining dependencies.
	stream::iter(models.into_iter().map(|model| Ok(model.solve(storage_backend))))
		.try_buffered(DEPENDENCY_SOLVE_CONCURRENCY)
		.try_collect()
		.await
}

pub mod animation;
pub mod audio;
pub mod flipbook;
pub mod image;
pub mod lut;
pub mod material;
pub mod mesh;
pub mod mips;
pub mod pipeline;
pub mod skeleton;

#[cfg(test)]
mod tests {
	use super::{
		animation::{Animation, AnimationModel},
		audio::Audio,
		image::Image,
		lut::Lut,
		material::{Material, MaterialModel, Shader, Variant, VariantModel},
		mesh::{Mesh, MeshModel, Primitive},
		skeleton::{Skeleton, SkeletonModel},
	};
	use crate::{Model, Resource};

	fn assert_resource_model<ResourceType, ModelType>()
	where
		ResourceType: Resource<Model = ModelType>,
		ModelType: Model,
	{
	}

	#[test]
	fn persisted_resource_class_tags_match_their_runtime_model_contract() {
		assert_resource_model::<Animation, AnimationModel>();
		assert_resource_model::<Audio, Audio>();
		assert_resource_model::<Image, Image>();
		assert_resource_model::<Lut, Lut>();
		assert_resource_model::<Material, MaterialModel>();
		assert_resource_model::<Variant, VariantModel>();
		assert_resource_model::<Shader, Shader>();
		assert_resource_model::<Primitive, Primitive>();
		assert_resource_model::<Mesh, MeshModel>();
		assert_resource_model::<Skeleton, SkeletonModel>();

		let tags = [
			(<AnimationModel as Model>::get_class(), "Animation"),
			(<Audio as Model>::get_class(), "Audio"),
			(<Image as Model>::get_class(), "Image"),
			(<Lut as Model>::get_class(), "Lut"),
			(<MaterialModel as Model>::get_class(), "Material"),
			(<VariantModel as Model>::get_class(), "Variant"),
			(<Shader as Model>::get_class(), "Shader"),
			(<Primitive as Model>::get_class(), "Primitive"),
			(<MeshModel as Model>::get_class(), "Mesh"),
			(<SkeletonModel as Model>::get_class(), "Skeleton"),
		];

		for (actual, expected) in tags {
			assert_eq!(actual, expected);
		}
	}
}
