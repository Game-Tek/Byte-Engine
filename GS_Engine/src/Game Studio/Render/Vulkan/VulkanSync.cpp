#include "VulkanSync.h"

#include "Vulkan.h"

VulkanFence::VulkanFence(VkDevice _Device) : VulkanObject(_Device)
{
	VkFenceCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
	
	GS_VK_CHECK(vkCreateFence(m_Device, &CreateInfo, ALLOCATOR, &Fence), "Failed to create Fence!")
}

VulkanFence::~VulkanFence()
{
	vkDestroyFence(m_Device, Fence, ALLOCATOR);
}

VulkanSemaphore::VulkanSemaphore(VkDevice _Device) : VulkanObject(_Device)
{
	VkSemaphoreCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO };

	GS_VK_CHECK(vkCreateSemaphore(m_Device, &CreateInfo, ALLOCATOR, &Semaphore), "Failed to create Semaphore!")
}

VulkanSemaphore::~VulkanSemaphore()
{
	vkDestroySemaphore(m_Device, Semaphore, ALLOCATOR);
}