#pragma once

#include "..\RenderPass.h"
#include "VulkanBase.h"

MAKE_VK_HANDLE(VkRenderPass)

class VulkanSwapchain;

GS_CLASS VulkanRenderPass final : public RenderPass, public VulkanObject
{
	Vk_RenderPass RenderPass;
public:
	VulkanRenderPass(VkDevice _Device, const RenderPassDescriptor & _RPD);
	~VulkanRenderPass();

	INLINE const Vk_RenderPass& GetVk_RenderPass() const { return RenderPass; }
};

class Vk_RenderPass : public VulkanObject
{
	VkRenderPass RenderPass = nullptr;
public:
	Vk_RenderPass(VkDevice _Device, const RenderPassDescriptor& _RPD);
	~Vk_RenderPass();

	INLINE VkRenderPass GetVkRenderPass() const { return RenderPass; }
};