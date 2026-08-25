//! Source-format implementations of [`super::AssetHandler`].

pub(crate) mod bema_handler {

	pub(crate) use super::bema::*;
}

pub(crate) mod handler {

	pub(crate) use super::super::{AssetHandler, BakeContext, LoadErrors};
}

pub(crate) mod manager {

	pub(crate) use crate::asset::manager::*;
}

pub(crate) use crate::asset::{
	BEADType, ContainerDefaultResource, ResourceId, container_default_resource, sanitize_material_name, store_model,
	store_model_owned,
};

pub mod bema;
pub mod besl;
pub mod exr;
pub mod fbx;
pub mod gltf;
pub mod ies;
pub mod lut;
pub mod ogg;
pub mod pipeline;
pub mod png;
pub mod wav;

pub use bema::*;
pub use besl::*;
pub use exr::*;
pub use fbx::*;
pub use gltf::*;
pub use ies::*;
pub use lut::*;
pub use ogg::*;
pub use pipeline::*;
pub use png::*;
pub use wav::*;
