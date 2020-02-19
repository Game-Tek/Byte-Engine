#include "VulkanFramebuffer.h"

#include "Vulkan.h"

#include "VulkanRenderPass.h"

#include "VulkanRenderDevice.h"

#include "VulkanRenderTarget.h"

VulkanFramebuffer::
VulkanFramebuffer(VulkanRenderDevice* _Device, const FramebufferCreateInfo& framebufferCreateInfo) : Framebuffer(
	framebufferCreateInfo)
{
	FVector<VkImageView> Result(framebufferCreateInfo.Images.getLength());

	for (uint8 i = 0; i < Result.getCapacity(); ++i)
	{
		Result.push_back(SCAST(VulkanRenderTargetBase*, framebufferCreateInfo.Images[i])->GetVkImageView());
	}

	for (uint8 i = 0; i < framebufferCreateInfo.ClearValues.getLength(); ++i)
	{
		auto c = VkClearValue{
			framebufferCreateInfo.ClearValues[i].R, framebufferCreateInfo.ClearValues[i].G,
			framebufferCreateInfo.ClearValues[i].B, framebufferCreateInfo.ClearValues[i].A
		};
		clearValues.push_back(c);
	}

	attachmentCount = framebufferCreateInfo.Images.getLength();

	VkFramebufferCreateInfo FramebufferCreateInfo = {VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
	FramebufferCreateInfo.attachmentCount = framebufferCreateInfo.Images.getLength();
	FramebufferCreateInfo.width = framebufferCreateInfo.Extent.Width;
	FramebufferCreateInfo.height = framebufferCreateInfo.Extent.Height;
	FramebufferCreateInfo.layers = 1;
	FramebufferCreateInfo.renderPass = static_cast<VulkanRenderPass*>(framebufferCreateInfo.RenderPass)
	                                   ->GetVKRenderPass().GetHandle();
	FramebufferCreateInfo.pAttachments = Result.getData();

	GS_VK_CHECK(
		vkCreateFramebuffer(_Device->GetVKDevice().GetVkDevice(), &FramebufferCreateInfo, ALLOCATOR, &framebuffer),
		"Failed to create framebuffer!");
}
