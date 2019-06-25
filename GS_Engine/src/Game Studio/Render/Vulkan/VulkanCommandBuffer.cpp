#include "VulkanCommandBuffer.h"

#include "Vulkan.h"

VulkanCommandBuffer::VulkanCommandBuffer(VkDevice _Device, VkCommandPool _CP) : VulkanObject(_Device)
{
	VkCommandBufferAllocateInfo CommandBufferAllocateInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO };
	CommandBufferAllocateInfo.commandPool = _CP;
	CommandBufferAllocateInfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
	CommandBufferAllocateInfo.commandBufferCount = (uint32_t)commandBuffers.size();

	GS_VK_CHECK(vkAllocateCommandBuffers(m_Device, &CommandBufferAllocateInfo, &CommandBuffer), "Failed to Allocate Command Buffer!")
}

VulkanCommandBuffer::~VulkanCommandBuffer()
{

}

void VulkanCommandBuffer::BeginRecording()
{
	VkCommandBufferBeginInfo BeginInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
	BeginInfo.flags = VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT;
	BeginInfo.pInheritanceInfo = nullptr; // Optional

	GS_VK_CHECK(vkBeginCommandBuffer(CommandBuffer, &BeginInfo), "Failed to begin Command Buffer!")
}

void VulkanCommandBuffer::EndRecording()
{
	GS_VK_CHECK(vkEndCommandBuffer(CommandBuffer), "Failed to end Command Buffer!")
}
