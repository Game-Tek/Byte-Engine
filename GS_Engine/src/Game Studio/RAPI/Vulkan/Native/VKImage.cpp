#include "VKImage.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKImageCreator::VKImageCreator(VKDevice* _Device, const VkImageCreateInfo* _VkICI) : VKObjectCreator<VkImage>(_Device)
{
	GS_VK_CHECK(vkCreateImage(m_Device->GetVkDevice(), _VkICI, ALLOCATOR, &Handle), "Failed to create Image!")
}

VKImage::~VKImage()
{
	vkDestroyImage(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}

VkMemoryRequirements VKImage::GetMemoryRequirements() const
{
	VkMemoryRequirements MemoryRequirements;
	vkGetImageMemoryRequirements(m_Device->GetVkDevice(), Handle, &MemoryRequirements);
	return MemoryRequirements;
}
