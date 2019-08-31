#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkImageView)

struct VkImageViewCreateInfo;

GS_STRUCT VKImageViewCreator final : VKObjectCreator<VkImageView>
{
	VKImageViewCreator(VKDevice* _Device, const VkImageViewCreateInfo * _VkIVCI);
};

GS_CLASS VKImageView final : public VKObject<VkImageView>
{
public:
	VKImageView(const VKImageViewCreator& _VKIVC) : VKObject<VkImageView>(_VKIVC)
	{
	}

	~VKImageView();
};