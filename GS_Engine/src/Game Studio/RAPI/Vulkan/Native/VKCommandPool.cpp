#include "VKCommandPool.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"
#include "vkQueue.h"
#include "VKCommandBuffer.h"

VKCommandPoolCreator::VKCommandPoolCreator(const VKDevice& _Device, const VkCommandPoolCreateInfo* _VkCPCI) : VKObject<VkCommandPool>(_Device)
{
	GS_VK_CHECK(vkCreateCommandPool(m_Device, _VkCPCI, ALLOCATOR, &Handle), "Failed to create Command Pool!")
}

VKCommandPool::~VKCommandPool()
{
	vkDestroyCommandPool(m_Device, Handle, ALLOCATOR);
}

VKCommandBufferCreator VKCommandPool::CreateCommandBuffer() const
{
	VkCommandBufferAllocateInfo VkCommandBufferAllocateInfo;
	VkCommandBufferAllocateInfo.sType = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO };
	VkCommandBufferAllocateInfo.commandBufferCount = 1;
	VkCommandBufferAllocateInfo.commandPool = Handle;

	return VKCommandBufferCreator(m_Device, &VkCommandBufferAllocateInfo);
}

void VKCommandPool::Reset() const
{
	vkResetCommandPool(m_Device, Handle, 0);
}
