#pragma once

#include "..\RenderPass.h"
#include "VulkanBase.h"

MAKE_VK_HANDLE(VkRenderPass)

class VulkanSwapchain;

GS_CLASS VulkanRenderPass final : public RenderPass, public VulkanObject
{
	VkRenderPass RenderPass;
public:
	VulkanRenderPass(VkDevice _Device, const RenderPassDescriptor & _RPD);
	~VulkanRenderPass();

	void AddSubPass() override final;

	INLINE VkRenderPass GetVkRenderPass() const { return RenderPass; }
};