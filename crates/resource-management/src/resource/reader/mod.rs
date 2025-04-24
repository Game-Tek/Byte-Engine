pub mod redb;

use std::fmt::Debug;

use crate::StreamDescription;

use super::resource_handler::{LoadTargets, ReadTargets};

/// The resource reader trait provides methods to read a single resource.
pub trait ResourceReader: Send + Sync + Debug {
	fn read_into<'b, 'c: 'b, 'a: 'b>(&'b mut self, stream_descriptions: Option<&'c [StreamDescription]>, read_target: ReadTargets<'a>) -> Result<LoadTargets<'a>, ()>;
}