//! Metal resource operations split by responsibility.

pub(in crate::metal) mod acceleration_structures;
mod commands;
mod staging;
mod swapchain;
mod synchronization;
