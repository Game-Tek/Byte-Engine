#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "Native/Vk_ImageView.h"
#include "Native/Vk_Memory.h"

GS_CLASS VulkanImage final : public Image
{
	Vk_Image m_Image;
	Vk_Memory ImageMemory;
	Vk_ImageView ImageView;

public:
	VulkanImage(const Vk_Device& _Device, const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, const ImageUse _ImgUse, LoadOperations _LO, StoreOperations _SO, ImageLayout _IL, ImageLayout _FL);

	INLINE VkImageView GetVkImageView() const { return ImageView; }
};
