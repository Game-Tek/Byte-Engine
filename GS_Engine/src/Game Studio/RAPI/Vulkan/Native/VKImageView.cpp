#include "VKImageView.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKImageViewCreator::
VKImageViewCreator(VKDevice* _Device, const VkImageViewCreateInfo* _VkIVCI) : VKObjectCreator<VkImageView>(_Device)
{
	GS_VK_CHECK(vkCreateImageView(m_Device->GetVkDevice(), _VkIVCI, ALLOCATOR, &Handle), "Failed to create Image View!")
}

VKImageView::~VKImageView()
{
	vkDestroyImageView(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
