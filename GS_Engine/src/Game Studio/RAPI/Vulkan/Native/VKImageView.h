#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkImageView)

struct VkImageViewCreateInfo;

struct VKImageViewCreator final : VKObjectCreator<VkImageView>
{
	VKImageViewCreator(VKDevice* _Device, const VkImageViewCreateInfo* _VkIVCI);
};

class VKImageView final : public VKObject<VkImageView>
{
public:
	VKImageView(const VKImageViewCreator& _VKIVC) : VKObject<VkImageView>(_VKIVC)
	{
	}

	~VKImageView();
};
