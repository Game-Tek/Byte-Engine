#include "VulkanCommandBuffer.h"

#include "Vulkan.h"

#include "VulkanRenderPass.h"
#include "VulkanFramebuffer.h"
#include "VulkanMesh.h"
#include "VulkanPipelines.h"
#include "VulkanBindings.h"

#include "VulkanRenderDevice.h"

VulkanCommandBuffer::VulkanCommandBuffer(VulkanRenderDevice* renderDevice, const CommandBufferCreateInfo& commandBufferCreateInfo):
	CommandBuffer(commandBufferCreateInfo)
{
	VkCommandPoolCreateInfo vk_command_pool_create_info{ VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
	vk_command_pool_create_info;

	vkCreateCommandPool(renderDevice->GetVkDevice(), &vk_command_pool_create_info, renderDevice->GetVkAllocationCallbacks(), &commandPool);

	VkCommandBufferAllocateInfo vk_command_buffer_allocate_info{ VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO };
	vk_command_buffer_allocate_info.commandPool = commandPool;
	vk_command_buffer_allocate_info.commandBufferCount = 1;
	vk_command_buffer_allocate_info.level = commandBufferCreateInfo.IsPrimary ? VK_COMMAND_BUFFER_LEVEL_PRIMARY : VK_COMMAND_BUFFER_LEVEL_SECONDARY;
	
	vkAllocateCommandBuffers(renderDevice->GetVkDevice(), &vk_command_buffer_allocate_info, &commandBuffer);
}

void VulkanCommandBuffer::BeginRecording(const BeginRecordingInfo& beginRecordingInfo)
{
	VkCommandBufferBeginInfo vk_command_buffer_begin_info{ VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
	vk_command_buffer_begin_info.flags = VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT;
	//Hint to primary buffer if this is secondary.
	vk_command_buffer_begin_info.pInheritanceInfo = nullptr;

	vkBeginCommandBuffer(commandBuffer, &vk_command_buffer_begin_info);
}

void VulkanCommandBuffer::EndRecording(const EndRecordingInfo& endRecordingInfo)
{
	vkEndCommandBuffer(commandBuffer);
}

void VulkanCommandBuffer::BeginRenderPass(const BeginRenderPassInfo& beginRenderPassInfo)
{
	VkRenderPassBeginInfo RenderPassBeginInfo = { VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO };
	RenderPassBeginInfo.renderPass = static_cast<VulkanRenderPass*>(beginRenderPassInfo.RenderPass)->GetVkRenderPass();
	RenderPassBeginInfo.pClearValues = static_cast<VulkanFramebuffer*>(beginRenderPassInfo.Framebuffer)->GetClearValues().getData();
	RenderPassBeginInfo.clearValueCount = static_cast<uint32>(static_cast<VulkanFramebuffer*>(beginRenderPassInfo.Framebuffer)->GetClearValues().getLength());
	RenderPassBeginInfo.framebuffer = static_cast<VulkanFramebuffer*>(beginRenderPassInfo.Framebuffer)->GetVkFramebuffer();
	RenderPassBeginInfo.renderArea.extent = Extent2DToVkExtent2D(beginRenderPassInfo.RenderArea);
	RenderPassBeginInfo.renderArea.offset = { 0, 0 };

	vkCmdBeginRenderPass(commandBuffer, &RenderPassBeginInfo, VK_SUBPASS_CONTENTS_INLINE);
}

void VulkanCommandBuffer::AdvanceSubPass(const AdvanceSubpassInfo& advanceSubpassInfo)
{
	vkCmdNextSubpass(commandBuffer, VK_SUBPASS_CONTENTS_INLINE);
}

void VulkanCommandBuffer::EndRenderPass(const EndRenderPassInfo& endRenderPassInfo)
{
	vkCmdEndRenderPass(commandBuffer);
}

void VulkanCommandBuffer::BindGraphicsPipeline(const BindGraphicsPipelineInfo& bindGraphicsPipelineInfo)
{
	VkViewport viewport;
	viewport.x = 0;
	viewport.y = 0;
	viewport.minDepth = 0;
	viewport.maxDepth = 1.0f;
	viewport.width = bindGraphicsPipelineInfo.RenderExtent.Width;
	viewport.height = bindGraphicsPipelineInfo.RenderExtent.Height;
	vkCmdSetViewport(commandBuffer, 0, 1, &viewport);

	vkCmdBindPipeline(commandBuffer, VK_PIPELINE_BIND_POINT_GRAPHICS, static_cast<VulkanGraphicsPipeline*>(bindGraphicsPipelineInfo.GraphicsPipeline)->GetVkGraphicsPipeline());
}

void VulkanCommandBuffer::BindComputePipeline(const BindComputePipelineInfo& bindComputePipelineInfo)
{
	vkCmdBindPipeline(commandBuffer, VK_PIPELINE_BIND_POINT_COMPUTE, static_cast<VulkanComputePipeline*>(bindComputePipelineInfo.Pipeline)->GetVkPipeline());
}

void VulkanCommandBuffer::BindMesh(const BindMeshInfo& bindMeshInfo)
{
	const auto mesh = static_cast<VulkanMesh*>(bindMeshInfo.Mesh);
	VkDeviceSize offset = 0;

	VkBuffer vertex_buffers = mesh->GetVertexBuffer().GetHandle();

	vkCmdBindVertexBuffers(commandBuffer, 0, 1, &vertex_buffers, &offset);
	vkCmdBindIndexBuffer(commandBuffer, mesh->GetIndexBuffer().GetHandle(), 0, VK_INDEX_TYPE_UINT16);
}

void VulkanCommandBuffer::UpdatePushConstant(const UpdatePushConstantsInfo& updatePushConstantsInfo)
{
	vkCmdPushConstants(commandBuffer, static_cast<VulkanGraphicsPipeline*>(updatePushConstantsInfo.Pipeline)->GetVkPipelineLayout(), VK_SHADER_STAGE_ALL_GRAPHICS, updatePushConstantsInfo.Offset, updatePushConstantsInfo.Size, updatePushConstantsInfo.Data);
}

void VulkanCommandBuffer::DrawIndexed(const DrawIndexedInfo& drawIndexedInfo)
{
	vkCmdDrawIndexed(commandBuffer, drawIndexedInfo.IndexCount, drawIndexedInfo.InstanceCount, 0, 0, 0);
}

void VulkanCommandBuffer::Dispatch(const DispatchInfo& dispatchInfo)
{
	vkCmdDispatch(commandBuffer, dispatchInfo.WorkGroups.Width, dispatchInfo.WorkGroups.Height, dispatchInfo.WorkGroups.Depth);
}

void VulkanCommandBuffer::BindBindingsSet(const BindBindingsSetInfo& bindBindingsSetInfo)
{
	Array<VkDescriptorSet, 8> descriptor_sets(bindBindingsSetInfo.BindingsSets->getLength());
	{
		uint8 i = 0;

		for (auto& e : descriptor_sets)
		{
			e = static_cast<VulkanBindingsSet*>((*bindBindingsSetInfo.BindingsSets)[i])->GetVkDescriptorSets()[bindBindingsSetInfo.BindingsSetIndex];
			++i;
		}
	}

	vkCmdBindDescriptorSets(commandBuffer, VK_PIPELINE_BIND_POINT_GRAPHICS,	static_cast<VulkanGraphicsPipeline*>(bindBindingsSetInfo.Pipeline)->GetVkPipelineLayout(), 0, descriptor_sets.getLength(), descriptor_sets.getData(), 0, 0);
}
