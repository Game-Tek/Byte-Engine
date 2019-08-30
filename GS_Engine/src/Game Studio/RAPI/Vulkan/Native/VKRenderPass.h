#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkRenderPass)

struct VkRenderPassCreateInfo;

GS_STRUCT VKRenderPassCreator : VKObjectCreator<VkRenderPass>
{
	VKRenderPassCreator(const VKDevice& _Device, const VkRenderPassCreateInfo* _VkRPCI);
};

GS_CLASS VKRenderPass final : public VKObject<VkRenderPass>
{
public:
	explicit VKRenderPass(const VKRenderPassCreator& _VkRPC) : VKObject<VkRenderPass>(_VkRPC)
	{
	}

	~VKRenderPass();
};