#pragma once

#include "GAL/Framebuffer.h"

#include "Vulkan.h"
#include "VulkanRenderDevice.h"

namespace GAL
{
	class VulkanFramebuffer final : public Framebuffer
	{
	public:
		VulkanFramebuffer() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, VulkanRenderPass renderPass, GTSL::Extent2D extent, GTSL::Range<const class VulkanTextureView*> textureViews) {
			VkFramebufferCreateInfo vkFramebufferCreateInfo{ VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
			vkFramebufferCreateInfo.width = extent.Width;
			vkFramebufferCreateInfo.height = extent.Height;
			vkFramebufferCreateInfo.layers = 1;
			vkFramebufferCreateInfo.renderPass = renderPass.GetVkRenderPass();
			vkFramebufferCreateInfo.attachmentCount = static_cast<GTSL::uint32>(textureViews.ElementCount());
			vkFramebufferCreateInfo.pAttachments = reinterpret_cast<const VkImageView*>(textureViews.begin());

			renderDevice->VkCreateFramebuffer(renderDevice->GetVkDevice(), &vkFramebufferCreateInfo, renderDevice->GetVkAllocationCallbacks(), &framebuffer);
			//setName(createInfo.RenderDevice, framebuffer, VK_OBJECT_TYPE_FRAMEBUFFER, createInfo.Name);
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyFramebuffer(renderDevice->GetVkDevice(), framebuffer, renderDevice->GetVkAllocationCallbacks());
			debugClear(framebuffer);
		}
		
		~VulkanFramebuffer() = default;


		[[nodiscard]] VkFramebuffer GetVkFramebuffer() const { return framebuffer; }
		[[nodiscard]] uint64_t GetHandle() const { return reinterpret_cast<uint64_t>(framebuffer); }

	private:
		VkFramebuffer framebuffer = nullptr;
	};
}