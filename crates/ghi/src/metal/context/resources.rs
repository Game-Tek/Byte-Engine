//! Metal resource operations split by responsibility.

pub(in crate::metal) mod acceleration_structures;
mod allocation;
mod commands;
mod descriptors;
mod images;
mod pipelines;
mod staging;
mod swapchain;
mod synchronization;
