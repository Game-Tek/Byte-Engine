//! DX12 resource operations split by responsibility.

#[path = "resources/acceleration_structures.rs"]
mod acceleration_structures;
#[path = "resources/buffers.rs"]
mod buffers;
#[path = "resources/commands.rs"]
mod commands;
#[path = "resources/descriptors.rs"]
mod descriptors;
#[path = "resources/images.rs"]
mod images;
#[path = "resources/io.rs"]
mod io;
#[path = "resources/pipelines.rs"]
mod pipelines;
#[path = "resources/staging.rs"]
mod staging;
