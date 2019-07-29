#include "RAPI/Vulkan/Vulkan.h"

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

void Vk_CommandBuffer::Begin(VkCommandBufferBeginInfo* _CBBI)
{
	GS_VK_CHECK(vkBeginCommandBuffer(CommandBuffer, _CBBI), "Failed to begin Command Buffer!")
}

void Vk_CommandBuffer::End()
{
	GS_VK_CHECK(vkEndCommandBuffer(CommandBuffer), "Failed to end Command Buffer!")
}