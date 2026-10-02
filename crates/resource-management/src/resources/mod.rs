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
			) -> $crate::r#async::BoxedFuture<'de, Result<$crate::Reference<$resource>, $crate::solver::SolveError>> {
				$crate::r#async::future(async move {
					let resource: $resource = $crate::from_slice(stored.resource())
						.map_err(|error| $crate::solver::SolveError::DeserializationFailed(error.to_string()))?;

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
) -> Result<Vec<T>, crate::solver::SolveError>
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

/// The `ModelSpace` struct brands geometry in the coordinates its asset was authored in.
///
/// Mesh vertices and bounds use it until an instance transform places them in the world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModelSpace;

/// The `ParentSpace` struct brands a skeleton node's local transform, which is relative to its parent node.
///
/// A root node's parent space is its skeleton's [`ModelSpace`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParentSpace;

pub mod animation;
pub mod audio;
pub mod flipbook;
pub mod image;
pub mod lut;
pub mod material;
pub mod mesh;
pub mod mips;
pub mod particle_system;
pub mod pipeline;
pub mod skeleton;
