#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkImageView)

struct VkImageViewCreateInfo;

struct GS_API VKImageViewCreator final : VKObjectCreator<VkImageView>
{
	VKImageViewCreator(VKDevice* _Device, const VkImageViewCreateInfo* _VkIVCI);
};

class GS_API VKImageView final : public VKObject<VkImageView>
{
public:
	VKImageView(const VKImageViewCreator& _VKIVC) : VKObject<VkImageView>(_VKIVC)
	{
	}

	~VKImageView();
};
