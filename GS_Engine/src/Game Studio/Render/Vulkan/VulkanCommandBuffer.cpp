#include "Vulkan.h"

#include "VulkanCommandBuffer.h"
#include "VulkanRenderPass.h"
#include "VulkanFramebuffer.h"
#include "VulkanPipelines.h"

VkExtent2D Extent2DToVkExtent2D(Extent2D _Extent)
{
	return { _Extent.Width, _Extent.Height };
}

VulkanCommandBuffer::VulkanCommandBuffer(VkDevice _Device, uint32 _QueueIndex) : CommandPool(_Device, _QueueIndex),
	CommandBuffer(_Device, CommandPool.GetVkCommandPool())
{
}

void VulkanCommandBuffer::BeginRecording()
{
	VkCommandBufferBeginInfo BeginInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
	BeginInfo.flags = VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT;
	//Hint to primary buffer if this is secondary.
	BeginInfo.pInheritanceInfo = nullptr;

	GS_VK_CHECK(vkBeginCommandBuffer(CommandBuffer.GetVkCommandBuffer(), &BeginInfo), "Failed to begin Command Buffer!")
}

void VulkanCommandBuffer::EndRecording()
{
	GS_VK_CHECK(vkEndCommandBuffer(CommandBuffer.GetVkCommandBuffer()), "Failed to end Command Buffer!")
}

void VulkanCommandBuffer::BeginRenderPass(const RenderPassBeginInfo& _RPBI)
{
	VkClearValue ClearColor = { 0.0f, 0.0f, 0.0f, 0.0f };

	VkRenderPassBeginInfo RenderPassBeginInfo = { VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO };
	RenderPassBeginInfo.renderPass = SCAST(VulkanRenderPass*, _RPBI.RenderPass)->GetVkRenderPass();
	RenderPassBeginInfo.pClearValues = &ClearColor;
	RenderPassBeginInfo.clearValueCount = 1;
	RenderPassBeginInfo.framebuffer = SCAST(VulkanFramebuffer*, _RPBI.Framebuffer)->GetVkFramebuffer();
	RenderPassBeginInfo.renderArea.extent = Extent2DToVkExtent2D(_RPBI.RenderArea);
	RenderPassBeginInfo.renderArea.offset = { 0, 0 };

	vkCmdBeginRenderPass(CommandBuffer.GetVkCommandBuffer(), &RenderPassBeginInfo, VK_SUBPASS_CONTENTS_INLINE);
}

void VulkanCommandBuffer::EndRenderPass(RenderPass* _RP)
{
	vkCmdEndRenderPass(CommandBuffer.GetVkCommandBuffer());
}

void VulkanCommandBuffer::BindGraphicsPipeline(GraphicsPipeline* _GP)
{
	vkCmdBindPipeline(CommandBuffer.GetVkCommandBuffer(), VK_PIPELINE_BIND_POINT_GRAPHICS, SCAST(VulkanGraphicsPipeline*, _GP)->GetVkGraphicsPipeline());
}

void VulkanCommandBuffer::BindComputePipeline(ComputePipeline* _CP)
{
	vkCmdBindPipeline(CommandBuffer.GetVkCommandBuffer(), VK_PIPELINE_BIND_POINT_COMPUTE, SCAST(VulkanComputePipeline*, _CP)->GetVkComputePipeline());
}

void VulkanCommandBuffer::DrawIndexed(const DrawInfo& _DI)
{
	vkCmdDrawIndexed(CommandBuffer.GetVkCommandBuffer(), _DI.IndexCount, _DI.InstanceCount, 0, 0, 0);
}

//  VK_COMMANDBUFFER

Vk_CommandBuffer::Vk_CommandBuffer(VkDevice _Device, VkCommandPool _CP) : VulkanObject(_Device)
{
	VkCommandBufferAllocateInfo CommandBufferAllocateInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO };
	CommandBufferAllocateInfo.commandPool = _CP;
	CommandBufferAllocateInfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
	CommandBufferAllocateInfo.commandBufferCount = 1;

	GS_VK_CHECK(vkAllocateCommandBuffers(m_Device, &CommandBufferAllocateInfo, &CommandBuffer), "Failed to allocate Command Buffer!")
}

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
