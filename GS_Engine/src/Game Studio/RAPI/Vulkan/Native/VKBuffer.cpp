#include "VKBuffer.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKBufferCreator::VKBufferCreator(VKDevice* _Device, const VkBufferCreateInfo* _VkBCI) : VKObjectCreator(_Device)
{
	GS_VK_CHECK(vkCreateBuffer(m_Device->GetVkDevice(), _VkBCI, ALLOCATOR, &Handle), "Failed to allocate Buffer!")
}

unsigned VKBuffer::BufferTypeToVkBufferUsageFlagBits(BufferType _BT)
{
	switch (_BT)
	{
	case BufferType::BUFFER_VERTEX: return VK_BUFFER_USAGE_VERTEX_BUFFER_BIT;
	case BufferType::BUFFER_INDEX: return VK_BUFFER_USAGE_INDEX_BUFFER_BIT;
	case BufferType::BUFFER_UNIFORM: return VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
	default: return VK_BUFFER_USAGE_FLAG_BITS_MAX_ENUM;
	}
}

VKBuffer::~VKBuffer()
{
	vkDestroyBuffer(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}

VkMemoryRequirements VKBuffer::GetMemoryRequirements() const
{
	VkMemoryRequirements MR;
	vkGetBufferMemoryRequirements(m_Device->GetVkDevice(), Handle, &MR);
	return MR;
}
