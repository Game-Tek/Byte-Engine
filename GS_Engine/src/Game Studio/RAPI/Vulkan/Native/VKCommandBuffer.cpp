#include "VKCommandBuffer.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"
#include "VKCommandPool.h"

//  VK_COMMANDBUFFER
VKCommandBufferCreator::VKCommandBufferCreator(VKDevice* _Device, const VkCommandBufferAllocateInfo* _VkCBCI) : VKObjectCreator(_Device)
{
	GS_VK_CHECK(vkAllocateCommandBuffers(m_Device->GetVkDevice(), _VkCBCI, &Handle), "Failed to allocate Command Buffer!")
}

void VKCommandBuffer::Free(const VKCommandPool& _CP) const
{
	vkFreeCommandBuffers(m_Device->GetVkDevice(), _CP.GetHandle(), 1, &Handle);
}

void VKCommandBuffer::Reset() const
{
	vkResetCommandBuffer(Handle, 0);
}

void VKCommandBuffer::Begin(VkCommandBufferBeginInfo* _CBBI)
{
	GS_VK_CHECK(vkBeginCommandBuffer(Handle, _CBBI), "Failed to begin Command Buffer!")
}

void VKCommandBuffer::End()
{
	GS_VK_CHECK(vkEndCommandBuffer(Handle), "Failed to end Command Buffer!")
}