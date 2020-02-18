#pragma once

#include "RAPI/RenderPass.h"

#include "Native/VKRenderPass.h"

class VulkanRenderPass final : public RAPI::RenderPass
{
	VKRenderPass RenderPass;

	static VKRenderPassCreator CreateInfo(VKDevice* _Device, const RenderPassDescriptor& _RPD);
public:
	VulkanRenderPass(VKDevice* _Device, const RenderPassDescriptor& _RPD);
	~VulkanRenderPass() = default;

	INLINE const VKRenderPass& GetVKRenderPass() const { return RenderPass; }
};
