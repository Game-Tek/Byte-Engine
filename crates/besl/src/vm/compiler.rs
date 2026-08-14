//! Lowers linked BESL syntax into executable VM instructions.

mod lowering;
mod support;

pub(super) use lowering::compile;
