#include "VulkanFramebuffer.h"

#include "Vulkan.h"

#include "VulkanRenderPass.h"

#include "VulkanRenderDevice.h"

#include "VulkanRenderTarget.h"

VulkanFramebuffer::VulkanFramebuffer(VulkanRenderDevice* vulkanRenderDevice, const FramebufferCreateInfo& framebufferCreateInfo) : Framebuffer(framebufferCreateInfo)
{
	FVector<VkImageView> result(framebufferCreateInfo.Images.getLength());

	for (uint8 i = 0; i < result.getCapacity(); ++i)
	{
		result.push_back(static_cast<VulkanRenderTargetBase*>(framebufferCreateInfo.Images[i])->GetVkImageView());
	}

	for (uint8 i = 0; i < framebufferCreateInfo.ClearValues.getLength(); ++i)
	{
		auto c = VkClearValue{ { { framebufferCreateInfo.ClearValues[i].R, framebufferCreateInfo.ClearValues[i].G, framebufferCreateInfo.ClearValues[i].B, framebufferCreateInfo.ClearValues[i].A } } };
		clearValues.push_back(c);
	}

	attachmentCount = framebufferCreateInfo.Images.getLength();

	VkFramebufferCreateInfo vk_framebuffer_create_info{ VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
	vk_framebuffer_create_info.attachmentCount = framebufferCreateInfo.Images.getLength();
	vk_framebuffer_create_info.width = framebufferCreateInfo.Extent.Width;
	vk_framebuffer_create_info.height = framebufferCreateInfo.Extent.Height;
	vk_framebuffer_create_info.layers = 1;
	vk_framebuffer_create_info.renderPass = static_cast<VulkanRenderPass*>(framebufferCreateInfo.RenderPass)->GetVkRenderPass();
	vk_framebuffer_create_info.pAttachments = result.getData();

	VK_CHECK(vkCreateFramebuffer(vulkanRenderDevice->GetVkDevice(), &vk_framebuffer_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &framebuffer));
}

void VulkanFramebuffer::Destroy(RenderDevice* renderDevice)
{
	const auto vk_render_device = static_cast<VulkanRenderDevice*>(renderDevice);
	vkDestroyFramebuffer(vk_render_device->GetVkDevice(), framebuffer, vk_render_device->GetVkAllocationCallbacks());
}
