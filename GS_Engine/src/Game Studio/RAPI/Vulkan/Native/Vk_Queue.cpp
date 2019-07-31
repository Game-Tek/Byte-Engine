#include "Vk_Queue.h"

#include "RAPI/Vulkan/Vulkan.h"

Vk_Queue::Vk_Queue(VkQueue _Queue, uint32 _Index) : Queue(_Queue), QueueIndex(_Index)
{
}

void Vk_Queue::Submit(const VkSubmitInfo* _SubmitInfo, VkFence _Fence) const
{
	GS_VK_CHECK(vkQueueSubmit(Queue, 1, _SubmitInfo, _Fence), "Failed to Submit!")
}

void Vk_Queue::Present(const VkPresentInfoKHR* _PresentInfo) const
{
	GS_VK_CHECK(vkQueuePresentKHR(Queue, _PresentInfo), "Failed to present!")
}

void Vk_Queue::Wait() const
{
	vkQueueWaitIdle(Queue);
}
