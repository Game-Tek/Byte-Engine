//! Skeletal animation evaluation and pose construction.
//!
//! Use [`sample_pose`] to turn imported animation data into global skeleton
//! matrices for a renderable. Then send those matrices through
//! [`crate::rendering::UpdatePose`].

pub mod skeletal;

pub use skeletal::sample_pose;
