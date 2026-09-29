//! Backend-independent GHI queue types.

use crate::WorkloadTypes;

pub struct QueueSelection {
	pub(crate) r#type: WorkloadTypes,
}

impl QueueSelection {
	pub fn new(r#type: WorkloadTypes) -> Self {
		Self { r#type }
	}
}
