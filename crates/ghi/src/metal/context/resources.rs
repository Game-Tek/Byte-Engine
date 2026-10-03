//! Metal resource operations split by responsibility.

pub(in crate::metal) mod acceleration_structures;
mod commands;
mod staging;
mod swapchain;
pub(in crate::metal) use swapchain::SWAPCHAIN_FORMAT;
mod synchronization;
