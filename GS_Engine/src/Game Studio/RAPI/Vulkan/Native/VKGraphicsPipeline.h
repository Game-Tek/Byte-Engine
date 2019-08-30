#pragma once

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FVector.hpp"

MAKE_VK_HANDLE(VkPipeline)

MAKE_VK_HANDLE(VkShaderModule)

struct VkGraphicsPipelineCreateInfo;

GS_STRUCT VKGraphicsPipelineCreator : VKObjectCreator<VkPipeline>
{
	VKGraphicsPipelineCreator(const VKDevice & _Device, const VkGraphicsPipelineCreateInfo * _VGPCI);
};

GS_CLASS VKGraphicsPipeline final : public VKObject<VkPipeline>
{
public:
	explicit VKGraphicsPipeline(const VKGraphicsPipelineCreator& _Vk_GPC) : VKObject<VkPipeline>(_Vk_GPC)
	{
	}

	~VKGraphicsPipeline();
};