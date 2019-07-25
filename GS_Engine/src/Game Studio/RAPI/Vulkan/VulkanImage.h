#pragma once

#include "Core.h"

#include "VulkanBase.h"
#include "RAPI/Image.h"

MAKE_VK_HANDLE(VkImage)
MAKE_VK_HANDLE(VkImageView)

struct VkExtent2D;
enum VkImageType;
enum VkFormat;

GS_CLASS Vk_Image final : public VulkanObject
{
	VkImage Image = nullptr;

public:
	Vk_Image(VkDevice _Device, VkExtent2D _Extent, VkImageType _Type, VkFormat _Format, VkImageUsageFlags _IUF);
	~Vk_Image();
};

enum VkImageViewType;
enum VkImageAspectFlagBits;
enum VkFormat;

GS_CLASS Vk_ImageView final : public VulkanObject
{
	VkImageView ImageView = nullptr;

public:
	Vk_ImageView(VkDevice _Device, VkImage _Image, VkImageViewType _IVT, VkFormat _Format, VkImageAspectFlagBits _IAFB);
	~Vk_ImageView();

	INLINE VkImageView GetVkImageView() const { return ImageView; }
};

GS_CLASS VulkanImage final : public Image
{
	Vk_ImageView ImageView;

public:
	VulkanImage();

	INLINE VkImageView GetVkImageView() const { return ImageView.GetVkImageView(); }
};
