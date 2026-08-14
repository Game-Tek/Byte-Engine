//! DX12 command recording operations split by responsibility.

#[path = "commands/compute.rs"]
mod compute;
#[path = "commands/descriptors.rs"]
mod descriptors;
#[path = "commands/pipelines.rs"]
mod pipelines;
#[path = "commands/raster.rs"]
mod raster;
#[path = "commands/ray_tracing.rs"]
mod ray_tracing;
#[path = "commands/submission.rs"]
mod submission;
