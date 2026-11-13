//! Source-format implementations of [`super::AssetHandler`].

pub(crate) use crate::asset::{
	ANIMATION_FRAGMENT_PREFIX, BEADType, ContainerDefaultResource, DEFAULT_ANIMATION_FRAGMENT, MATERIAL_FRAGMENT_PREFIX,
	ResourceId, SKELETON_FRAGMENT, commit_mesh, generated_skeleton_id, handler, manager, mesh_settings,
	missing_generated_material, names_generated_material, sanitize_material_name, select_unfragmented_resource,
	store_imported_image, store_model, store_model_owned,
};

pub mod audio_graph;
pub mod bema;
pub mod besl;
pub mod environment;
pub mod exr;
pub mod fbx;
pub mod flipbook;
pub mod font;
pub mod fusion;
pub mod gltf;
pub mod ies;
pub mod image;
pub mod level;
pub mod lut;
pub mod ogg;
pub mod particles;
pub mod pipeline;
pub mod wav;

pub use audio_graph::*;
pub use bema::*;
pub use besl::*;
pub use environment::*;
pub use exr::*;
pub use fbx::*;
pub use flipbook::*;
pub use font::*;
pub use fusion::*;
pub use gltf::*;
pub use ies::*;
pub use self::image::*;
pub use level::*;
pub use lut::*;
pub use ogg::*;
pub use particles::*;
pub use pipeline::*;
pub use wav::*;
