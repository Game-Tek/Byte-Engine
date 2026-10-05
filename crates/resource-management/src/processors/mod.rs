//! Process source data into engine resource representations.
//!
//! Each submodule is one concrete processor that asset handlers call after decoding their source format.

pub mod audio;
pub mod image;
pub mod lut;
pub mod mesh;

pub use audio::*;
pub use image::*;
pub use lut::*;
pub use mesh::*;
