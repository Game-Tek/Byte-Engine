//! Source-format implementations of [`super::AssetHandler`].

pub(crate) mod handler {

	pub(crate) use super::super::{AssetHandler, BakeContext, LoadErrors};
}

pub(crate) mod manager {

	pub(crate) use crate::asset::manager::*;
}

pub(crate) use crate::asset::{
	ANIMATION_FRAGMENT_PREFIX, BEADType, ContainerDefaultResource, DEFAULT_ANIMATION_FRAGMENT, ResourceId, SKELETON_FRAGMENT,
	commit_mesh, generated_skeleton_id, sanitize_material_name, select_unfragmented_resource, store_model, store_model_owned,
};

pub mod bema;
pub mod besl;
pub mod environment;
pub mod exr;
pub mod fbx;
pub mod flipbook;
pub mod gltf;
pub mod ies;
pub mod lut;
pub mod ogg;
pub mod particles;
pub mod pipeline;
pub mod png;
pub mod wav;

pub use bema::*;
pub use besl::*;
pub use environment::*;
pub use exr::*;
pub use fbx::*;
pub use flipbook::*;
pub use gltf::*;
pub use ies::*;
pub use lut::*;
pub use ogg::*;
pub use particles::*;
pub use pipeline::*;
pub use png::*;
pub use wav::*;
