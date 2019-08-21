#include "Vk_CommandPool.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"
#include "Vk_Queue.h"

Vk_CommandPool::Vk_CommandPool(const Vk_Device& _Device, const Vk_Queue& _Queue, unsigned _CPF) : VulkanObject(_Device)
{
	VkCommandPoolCreateInfo CommandPoolCreateInfo = { VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
	CommandPoolCreateInfo.queueFamilyIndex = _Queue.GetQueueIndex();
	CommandPoolCreateInfo.flags = _CPF;

	GS_VK_CHECK(vkCreateCommandPool(m_Device, &CommandPoolCreateInfo, ALLOCATOR, &CommandPool), "Failed to create Command Pool!")
}

Vk_CommandPool::~Vk_CommandPool()
{
	vkDestroyCommandPool(m_Device, CommandPool, ALLOCATOR);
}

void Vk_CommandPool::Reset() const
{
	vkResetCommandPool(m_Device, CommandPool, 0);
}
