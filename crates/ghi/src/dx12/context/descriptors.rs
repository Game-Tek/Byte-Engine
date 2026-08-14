//! DX12 descriptor operations split by responsibility.

#[path = "descriptors/clear_copies.rs"]
mod clear_copies;
#[path = "descriptors/descriptor_heaps.rs"]
mod descriptor_heaps;
#[path = "descriptors/hlsl_reflection.rs"]
mod hlsl_reflection;
#[path = "descriptors/materialization.rs"]
mod materialization;
#[path = "descriptors/pipeline_layout.rs"]
mod pipeline_layout;
#[path = "descriptors/view_descriptors.rs"]
mod view_descriptors;
