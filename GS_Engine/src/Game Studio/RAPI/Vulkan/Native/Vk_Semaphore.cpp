#include "Vk_Semaphore.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "Vk_Device.h"

Vk_Semaphore::Vk_Semaphore(const Vk_Device& _Device) : VulkanObject(_Device)
{
	VkSemaphoreCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO };

	GS_VK_CHECK(vkCreateSemaphore(m_Device, &CreateInfo, ALLOCATOR, &Semaphore), "Failed to create Semaphore!")
}

Vk_Semaphore::~Vk_Semaphore()
{
	vkDestroySemaphore(m_Device, Semaphore, ALLOCATOR);
}
