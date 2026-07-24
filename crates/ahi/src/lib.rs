//! Use the audio hardware interface (AHI) to send engine audio to a platform device.

#![feature(trait_alias)]

pub mod audio_hardware_interface;
pub mod os;

pub use os::Device;
