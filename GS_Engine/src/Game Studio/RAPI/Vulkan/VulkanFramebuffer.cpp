#include "VulkanFramebuffer.h"

#include "Vulkan.h"

#include "VulkanRenderPass.h"

#include "VulkanRenderDevice.h"

#include "VulkanRenderTarget.h"

VulkanFramebuffer::VulkanFramebuffer(VulkanRenderDevice* vulkanRenderDevice, const FramebufferCreateInfo& framebufferCreateInfo) : Framebuffer(framebufferCreateInfo)
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

	VkFramebufferCreateInfo vk_framebuffer_create_info{VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
	vk_framebuffer_create_info.attachmentCount = framebufferCreateInfo.Images.getLength();
	vk_framebuffer_create_info.width = framebufferCreateInfo.Extent.Width;
	vk_framebuffer_create_info.height = framebufferCreateInfo.Extent.Height;
	vk_framebuffer_create_info.layers = 1;
	vk_framebuffer_create_info.renderPass = static_cast<VulkanRenderPass*>(framebufferCreateInfo.RenderPass)->GetVkRenderPass();
	vk_framebuffer_create_info.pAttachments = Result.getData();

	GS_VK_CHECK(vkCreateFramebuffer(vulkanRenderDevice->GetVkDevice(), &vk_framebuffer_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &framebuffer), "Failed to create framebuffer!");
}
