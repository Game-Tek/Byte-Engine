#pragma once

#include "Core.h"
#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkImage)

enum VkImageType;
enum VkFormat;
enum VkImageUsageFlags;

struct VkExtent2D;

GS_CLASS Vk_Image final : public VulkanObject
{
	VkImage Image = nullptr;

public:
	Vk_Image(const Vk_Device& _Device, VkExtent2D _Extent,VkImageType _Type, VkFormat _Format, VkImageUsageFlags _IUF);
	~Vk_Image();

	INLINE operator VkImage() const { return Image; }
};
