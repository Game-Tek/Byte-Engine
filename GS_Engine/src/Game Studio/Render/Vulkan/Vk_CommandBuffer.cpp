#include "Vk_CommandBuffer.h"

#include "Vulkan.h"

//  VK_COMMANDBUFFER

Vk_CommandBuffer::Vk_CommandBuffer(VkDevice _Device, VkCommandPool _CP) : VulkanObject(_Device)
{
	VkCommandBufferAllocateInfo CommandBufferAllocateInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO };
	CommandBufferAllocateInfo.commandPool = _CP;
	CommandBufferAllocateInfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
	CommandBufferAllocateInfo.commandBufferCount = 1;

	GS_VK_CHECK(vkAllocateCommandBuffers(m_Device, &CommandBufferAllocateInfo, &CommandBuffer), "Failed to allocate Command Buffer!")
}


//  VK_COMMANDPOOL

Vk_CommandPool::Vk_CommandPool(VkDevice _Device, uint32 _QueueIndex) : VulkanObject(_Device)
{
	VkCommandPoolCreateInfo CreatePoolInfo = { VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
	CreatePoolInfo.queueFamilyIndex = _QueueIndex;

	GS_VK_CHECK(vkCreateCommandPool(_Device, &CreatePoolInfo, ALLOCATOR, &CommandPool), "Failed to create Command Pool!")
}

Vk_CommandPool::~Vk_CommandPool()
{
	vkDestroyCommandPool(m_Device, CommandPool, ALLOCATOR);
}
