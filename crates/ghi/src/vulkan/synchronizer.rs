use ash::vk;

use crate::synchronizer::SynchronizerHandle;

#[derive(Clone)]
pub struct Synchronizer {
	pub next: Option<SynchronizerHandle>,
	pub signaled: bool,
	/// Whether the fence is signaled or has a pending operation that will signal it; host waits on an unarmed fence never return.
	pub armed: bool,
	pub fence: vk::Fence,
	pub semaphore: vk::Semaphore,
}
