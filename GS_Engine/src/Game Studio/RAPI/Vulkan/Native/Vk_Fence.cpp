#include "Vk_Fence.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

Vk_Fence::Vk_Fence(const Vk_Device& _Device, bool _InitSignaled) : VulkanObject(_Device)
{
	VkFenceCreateInfo FenceCreateInfo = { VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
	FenceCreateInfo.flags = _InitSignaled;

	GS_VK_CHECK(vkCreateFence(m_Device, &FenceCreateInfo, ALLOCATOR, &Fence), "Failed to create Fence!");
}

Vk_Fence::~Vk_Fence()
{
	vkDestroyFence(m_Device, Fence, ALLOCATOR);
}

bool Vk_Fence::GetStatus() const
{
	const VkResult Result = vkGetFenceStatus(m_Device, Fence);

	switch (Result)
	{
	case VK_SUCCESS: return true;
	case VK_NOT_READY: return false;
	default: return false;
	}
}