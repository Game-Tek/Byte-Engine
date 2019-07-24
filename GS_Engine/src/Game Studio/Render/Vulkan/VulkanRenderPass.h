#pragma once

#include "VulkanBase.h"
#include "Render/RenderPass.h"


MAKE_VK_HANDLE(VkRenderPass)

GS_CLASS Vk_RenderPass final : public VulkanObject
{
	VkRenderPass RenderPass = nullptr;
public:
	Vk_RenderPass(VkDevice _Device, const RenderPassDescriptor& _RPD);
	~Vk_RenderPass();

	INLINE VkRenderPass GetVkRenderPass() const { return RenderPass; }

	INLINE operator VkRenderPass() const { return RenderPass; }
};

GS_CLASS VulkanRenderPass final : public RenderPass
{
	Vk_RenderPass RenderPass;
public:
	VulkanRenderPass(VkDevice _Device, const RenderPassDescriptor & _RPD);
	~VulkanRenderPass() = default;

	INLINE const Vk_RenderPass& GetVk_RenderPass() const { return RenderPass; }
};