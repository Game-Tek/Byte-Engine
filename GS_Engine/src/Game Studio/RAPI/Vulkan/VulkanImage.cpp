#include "VulkanImage.h"

#include "Vulkan.h"

#include "Native/VKDevice.h"
#include "Native/VKImage.h"

VulkanImageBase::VulkanImageBase(const Extent2D _ImgExtent, const Format _ImgFormat, const ImageType _ImgType, const ImageDimensions _ID) : Image(_ImgExtent, _ImgFormat, _ImgType, _ID)
{
}

VKImageCreator VulkanImage::CreateVKImageCreator(VKDevice* _Device, const Extent2D _ImgExtent,
	const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, ImageUse _ImgUse)
{
	VkImageCreateInfo ImageCreateInfo = { VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO };
	ImageCreateInfo.format = FormatToVkFormat(_ImgFormat);
	ImageCreateInfo.arrayLayers = 1;
	ImageCreateInfo.extent = { _ImgExtent.Width, _ImgExtent.Height, 0 };
	ImageCreateInfo.imageType = ImageDimensionsToVkImageType(_ID);
	ImageCreateInfo.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
	ImageCreateInfo.usage = VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_SAMPLED_BIT;
	ImageCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
	ImageCreateInfo.samples = VK_SAMPLE_COUNT_1_BIT;

	return VKImageCreator(_Device, &ImageCreateInfo);
}

VKMemoryCreator VulkanImage::CreateVKMemoryCreator(VKDevice* _Device, const VKImage& _Image)
{
	const auto MemoryRequirements = _Image.GetMemoryRequirements();

	VkMemoryAllocateInfo MemoryAllocateInfo = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	MemoryAllocateInfo.allocationSize = MemoryRequirements.size;
	MemoryAllocateInfo.memoryTypeIndex = _Device->FindMemoryType(MemoryRequirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);

	return VKMemoryCreator(_Device, &MemoryAllocateInfo);
}

VKImageViewCreator VulkanImage::CreateVKImageViewCreator(VKDevice* _Device, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType,	const VKImage& _Image)
{
	VkImageViewCreateInfo ImageViewCreateInfo = { VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
	ImageViewCreateInfo.format = FormatToVkFormat(_ImgFormat);
	ImageViewCreateInfo.image = _Image.GetHandle();
	ImageViewCreateInfo.viewType = ImageDimensionsToVkImageViewType(_ID);
	ImageViewCreateInfo.subresourceRange.aspectMask = ImageTypeToVkImageAspectFlagBits(_ImgType);
	ImageViewCreateInfo.subresourceRange.baseArrayLayer = 0;
	ImageViewCreateInfo.subresourceRange.baseMipLevel = 0;
	ImageViewCreateInfo.subresourceRange.layerCount = 1;
	ImageViewCreateInfo.subresourceRange.levelCount = 1;

	return VKImageViewCreator(_Device, &ImageViewCreateInfo);
}

VulkanImage::VulkanImage(VKDevice* _Device, const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, const ImageUse _ImgUse) :
	VulkanImageBase(_ImgExtent, _ImgFormat, _ImgType, _ID), 
	m_Image(CreateVKImageCreator(_Device, _ImgExtent, _ImgFormat, _ID, _ImgType, _ImgUse)),
	ImageMemory(CreateVKMemoryCreator(_Device, m_Image)),
	ImageView(CreateVKImageViewCreator(_Device, _ImgFormat, _ID, _ImgType, m_Image))
{
}