//! Portable VM value parsing, construction, encoding, and numeric semantics.

use super::*;

mod access;
mod operations;
mod representation;
mod serialization;

pub(crate) use access::*;
pub(crate) use operations::*;
pub use representation::Value;
pub(crate) use serialization::*;
