use ash::vk;

use crate::synchronizer::SynchronizerHandle;

#[derive(Clone)]
pub struct Synchronizer {
	pub next: Option<SynchronizerHandle>,

	pub signaled: bool,

	pub fence: vk::Fence,
	pub semaphore: vk::Semaphore,
}
