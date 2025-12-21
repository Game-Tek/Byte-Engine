//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

pub mod render_pass;

pub use render_pass::RenderPass as SimpleRenderPass;

use core::slice::SlicePattern;
use std::{collections::{hash_map::Entry, VecDeque}, sync::Arc};

use besl::ParserNode;
use ghi::{command_buffer::{BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecordable as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _}, device::Device as _, frame::Frame, Device};
use math::Matrix4;
use resource_management::{asset::material_asset_handler::ProgramGenerator, shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator};
use utils::{hash::{HashMap, HashMapExt}, json::{self, JsonContainerTrait as _, JsonValueTrait as _}, sync::RwLock, Box, Extent};

use crate::{camera::Camera, core::{entity::{self, EntityBuilder}, listener::{CreateEvent, Listener}, Entity, EntityHandle}, gameplay::Transformable, rendering::{common_shader_generator::CommonShaderScope, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, render_pass::{RenderPass, RenderPassBuilder, RenderPassCommand}, renderable::mesh::MeshSource, utils::{MeshBuffersStats, MeshStats}, RenderableMesh}};
