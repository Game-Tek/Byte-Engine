#include "Vulkan.h"

#include "Vk_CommandBuffer.h"

//  VK_COMMANDBUFFER

Vk_CommandBuffer::Vk_CommandBuffer(VkDevice _Device, VkCommandPool _CP) : VulkanObject(_Device)
{
	VkCommandBufferAllocateInfo CommandBufferAllocateInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO };
	CommandBufferAllocateInfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
	CommandBufferAllocateInfo.commandPool = _CP;
	CommandBufferAllocateInfo.commandBufferCount = 1;

	GS_VK_CHECK(vkAllocateCommandBuffers(m_Device, &CommandBufferAllocateInfo, &CommandBuffer), "Failed to allocate Command Buffer!")
}

void Vk_CommandBuffer::Free(VkCommandPool _CP)
{
	vkFreeCommandBuffers(m_Device, _CP, 1, &CommandBuffer);
}

void Vk_CommandBuffer::Submit(VkQueue _Queue, VkFence _Fence)
{
	VkSubmitInfo SubmitInfo = { VK_STRUCTURE_TYPE_SUBMIT_INFO };
	SubmitInfo.commandBufferCount = 1;
	SubmitInfo.pCommandBuffers = &CommandBuffer;

	vkQueueSubmit(_Queue, 1, &SubmitInfo, _Fence);
}


//  VK_COMMANDPOOL
Vk_CommandPool::Vk_CommandPool(VkDevice _Device, uint32 _QueueIndex, VkCommandPoolCreateFlagBits _CPF) : VulkanObject(_Device)
{
	VkCommandPoolCreateInfo CommandPoolCreateInfo = { VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
	CommandPoolCreateInfo.queueFamilyIndex = _QueueIndex;
	CommandPoolCreateInfo.flags = _CPF;

	GS_VK_CHECK(vkCreateCommandPool(_Device, &CommandPoolCreateInfo, ALLOCATOR, &CommandPool), "Failed to create Command Pool!")
}

Vk_CommandPool::~Vk_CommandPool()
{
	vkDestroyCommandPool(m_Device, CommandPool, ALLOCATOR);
}
