#pragma once

#include "RAPI/RenderPass.h"

#include "Native/VKRenderPass.h"

class VulkanRenderPass final : public RAPI::RenderPass
{
	VKRenderPass RenderPass;

	static VKRenderPassCreator CreateInfo(VkDevice* _Device, const RAPI::RenderPassDescriptor& _RPD);
public:
	VulkanRenderPass(VkDevice* _Device, const RAPI::RenderPassDescriptor& _RPD);
	~VulkanRenderPass() = default;

	INLINE const VKRenderPass& GetVKRenderPass() const { return RenderPass; }
};
