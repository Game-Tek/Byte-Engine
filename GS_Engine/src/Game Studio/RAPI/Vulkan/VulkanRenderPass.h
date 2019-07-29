#pragma once

#include "VulkanBase.h"
#include "RAPI/RenderPass.h"
#include "Native/Vk_RenderPass.h"

GS_CLASS VulkanRenderPass final : public RenderPass
{
	Vk_RenderPass RenderPass;
public:
	VulkanRenderPass(VkDevice _Device, const RenderPassDescriptor & _RPD);
	~VulkanRenderPass() = default;

	INLINE const Vk_RenderPass& GetVk_RenderPass() const { return RenderPass; }
};