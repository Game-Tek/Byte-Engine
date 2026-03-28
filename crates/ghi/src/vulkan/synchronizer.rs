use ash::vk;

use crate::{synchronizer::SynchronizerHandle, HandleLike, Next};

#[derive(Clone)]
pub(crate) struct Synchronizer {
	pub next: Option<SynchronizerHandle>,

	pub signaled: bool,

	pub fence: vk::Fence,
	pub semaphore: vk::Semaphore,
}
