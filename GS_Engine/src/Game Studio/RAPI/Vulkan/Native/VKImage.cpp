#include "VKImage.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKImageCreator::VKImageCreator(const VKDevice& _Device, const VkImageCreateInfo* _VkICI) : VKObjectCreator<VkImage>(_Device)
{
	GS_VK_CHECK(vkCreateImage(m_Device, _VkICI, ALLOCATOR, &Handle), "Failed to create Image!")
}

VKImage::~VKImage()
{
	vkDestroyImage(m_Device, Handle, ALLOCATOR);
}

VkMemoryRequirements VKImage::GetMemoryRequirements() const
{
	VkMemoryRequirements MemoryRequirements;
	vkGetImageMemoryRequirements(m_Device, Handle, &MemoryRequirements);
	return MemoryRequirements;
}

