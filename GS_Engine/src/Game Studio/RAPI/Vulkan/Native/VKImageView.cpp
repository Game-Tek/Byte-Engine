#include "VKImageView.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKImageViewCreator::VKImageViewCreator(const VKDevice& _Device, const VkImageViewCreateInfo* _VkIVCI) : VKObjectCreator<VkImageView>(_Device)
{
	GS_VK_CHECK(vkCreateImageView(m_Device, _VkIVCI, ALLOCATOR, &Handle), "Failed to create Image View!")
}

VKImageView::~VKImageView()
{
	vkDestroyImageView(m_Device, Handle, ALLOCATOR);
}
