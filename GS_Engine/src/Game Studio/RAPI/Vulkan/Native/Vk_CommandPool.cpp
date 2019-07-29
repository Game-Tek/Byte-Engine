#include "Vk_CommandPool.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "RAPI/Vulkan/VulkanRenderer.h"

Vk_CommandPool::Vk_CommandPool(const Vk_Device& _Device, const Vk_Queue& _Queue, VkCommandPoolCreateFlagBits _CPF) : VulkanObject(_Device)
{
	VkCommandPoolCreateInfo CommandPoolCreateInfo = { VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
	CommandPoolCreateInfo.queueFamilyIndex = _Queue.GetQueueFamilyIndex();
	CommandPoolCreateInfo.flags = _CPF;

	GS_VK_CHECK(vkCreateCommandPool(_Device, &CommandPoolCreateInfo, ALLOCATOR, &CommandPool), "Failed to create Command Pool!")
}

Vk_CommandPool::~Vk_CommandPool()
{
	vkDestroyCommandPool(m_Device, CommandPool, ALLOCATOR);
}
