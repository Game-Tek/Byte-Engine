//! The `vm` module compiles and executes lexed BESL programs for deterministic host-side evaluation.

use crate::lexer::{BindingTypes, Expressions, NodeReference, Nodes, Operators};

mod buffer;
mod compiler;
mod error;
mod execution;
mod instruction;
mod layout;
mod program;
mod resources;
mod texture;
mod value;

pub use buffer::Buffer;
pub use error::VmError;
pub use half::f16;
use instruction::*;
pub use layout::{
	BufferLayout, BufferMemberLayout, DescriptorLayout, ResourceSlot, ValueType, builtin_position_slot, input_slot, output_slot,
};
use layout::{PUSH_CONSTANT_SLOT, dynamic_resource_slot};
use program::{ExecutableFunction, ExecutionFrame, ExecutionState};
pub use program::{ExecutableProgram, ExecutionConfig, SpecializationValues};
pub use resources::{DescriptorBindings, MeshOutputs, TaskOutputs, WorkgroupState};
pub use texture::{Sampler, SamplerReductionMode, Texture};
pub use value::Value;
use value::*;

#[cfg(test)]
mod tests;
