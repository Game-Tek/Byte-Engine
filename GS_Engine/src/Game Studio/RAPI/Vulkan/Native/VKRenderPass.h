#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkRenderPass)

struct VkRenderPassCreateInfo;

struct GS_API VKRenderPassCreator : VKObjectCreator<VkRenderPass>
{
	VKRenderPassCreator(VKDevice* _Device, const VkRenderPassCreateInfo* _VkRPCI);
};

class GS_API VKRenderPass final : public VKObject<VkRenderPass>
{
public:
	explicit VKRenderPass(const VKRenderPassCreator& _VkRPC) : VKObject<VkRenderPass>(_VkRPC)
	{
	}

	~VKRenderPass();
};