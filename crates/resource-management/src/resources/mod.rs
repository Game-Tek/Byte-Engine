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

		impl<'de> $crate::Solver<'de, $crate::Reference<$resource>> for $crate::ReferenceModel<$resource> {
			/// Restores direct resource metadata while retaining its binary-data reader.
			fn solve(
				self,
				storage_backend: &dyn $crate::resource::ReadStorageBackend,
			) -> Result<$crate::Reference<$resource>, $crate::solver::SolveErrors> {
				let (stored, reader) = storage_backend
					.read(self.id())
					.ok_or($crate::solver::SolveErrors::StorageError)?;
				let resource: $resource = $crate::from_slice(stored.resource())
					.map_err(|error| $crate::solver::SolveErrors::DeserializationFailed(error.to_string()))?;

				Ok($crate::Reference::from_model(self, resource, reader))
			}
		}
	};
}

pub(crate) use impl_direct_resource;

pub mod animation;
pub mod audio;
pub mod image;
pub mod lut;
pub mod material;
pub mod mesh;
pub mod mips;
pub mod skeleton;

#[cfg(test)]
mod tests {
	use super::{
		animation::{Animation, AnimationModel},
		audio::Audio,
		image::Image,
		lut::Lut,
		material::{Material, MaterialModel, Shader, Variant, VariantModel},
		mesh::{Mesh, MeshModel, Primitive, PrimitiveModel},
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
		assert_resource_model::<Primitive, PrimitiveModel>();
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
			(<PrimitiveModel as Model>::get_class(), "Primitive"),
			(<MeshModel as Model>::get_class(), "Mesh"),
			(<SkeletonModel as Model>::get_class(), "Skeleton"),
		];

		for (actual, expected) in tags {
			assert_eq!(actual, expected);
		}
	}
}
