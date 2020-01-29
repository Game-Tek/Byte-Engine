#include "VKSemaphore.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "VKDevice.h"

VKSemaphoreCreator::
VKSemaphoreCreator(VKDevice* _Device, const VkSemaphoreCreateInfo* _VkSCI) : VKObjectCreator<VkSemaphore>(_Device)
{
	GS_VK_CHECK(vkCreateSemaphore(m_Device->GetVkDevice(), _VkSCI, ALLOCATOR, &Handle), "Failed to create Semaphore!")
}

VKSemaphore::~VKSemaphore()
{
	vkDestroySemaphore(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}
