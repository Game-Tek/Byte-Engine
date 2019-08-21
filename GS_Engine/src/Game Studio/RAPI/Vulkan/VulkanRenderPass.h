#pragma once

#include "RAPI/RenderPass.h"

#include "Native/Vk_RenderPass.h"

GS_CLASS VulkanRenderPass final : public RenderPass
{
	Vk_RenderPass RenderPass;

	static Vk_RenderPassCreateInfo CreateInfo(const Vk_Device& _Device, const RenderPassDescriptor& _RPD);
public:
	VulkanRenderPass(const Vk_Device& _Device, const RenderPassDescriptor & _RPD);
	~VulkanRenderPass() = default;

	INLINE const Vk_RenderPass& GetVk_RenderPass() const { return RenderPass; }
};