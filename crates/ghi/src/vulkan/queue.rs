use ash::vk;

pub(crate) struct Queue {
	pub(crate) vk_queue: vk::Queue,
    pub(crate) queue_family_index: u32,
    pub(crate) queue_index: u32,
}
