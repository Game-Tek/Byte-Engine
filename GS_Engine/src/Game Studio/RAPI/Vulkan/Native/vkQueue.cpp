#include "vkQueue.h"

#include "RAPI/Vulkan/Vulkan.h"

vkQueue::vkQueue(VkQueue _Queue, uint32 _Index) : Queue(_Queue), QueueIndex(_Index)
{
}

void vkQueue::Submit(const VkSubmitInfo* _SubmitInfo, VkFence _Fence) const
{
	GS_VK_CHECK(vkQueueSubmit(Queue, 1, _SubmitInfo, _Fence), "Failed to Submit!")
}

void vkQueue::Present(const VkPresentInfoKHR* _PresentInfo) const
{
	GS_VK_CHECK(vkQueuePresentKHR(Queue, _PresentInfo), "Failed to present!")
}

void vkQueue::Wait() const
{
	vkQueueWaitIdle(Queue);
}
