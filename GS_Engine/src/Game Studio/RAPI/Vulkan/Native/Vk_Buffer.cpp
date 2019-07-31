#include "Vk_Buffer.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

unsigned Vk_Buffer::BufferTypeToVkBufferUsageFlagBits(BufferType _BT)
{

	switch (_BT)
	{
	case BufferType::BUFFER_VERTEX:		return VK_BUFFER_USAGE_VERTEX_BUFFER_BIT;
	case BufferType::BUFFER_INDEX:		return VK_BUFFER_USAGE_INDEX_BUFFER_BIT;
	case BufferType::BUFFER_UNIFORM:	return VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
	default:							return VK_BUFFER_USAGE_FLAG_BITS_MAX_ENUM;
	}
}

Vk_Buffer::Vk_Buffer(const Vk_Device& _Device, uint32 _BufferUsage, size_t _Size) : VulkanObject(_Device)
{
	VkBufferCreateInfo BufferCreateInfo = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
	BufferCreateInfo.size = _Size;
	BufferCreateInfo.usage = _BufferUsage;
	BufferCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	GS_VK_CHECK(vkCreateBuffer(m_Device, &BufferCreateInfo, ALLOCATOR, &Buffer), "Failed to allocate Buffer!")
}

Vk_Buffer::~Vk_Buffer()
{
	vkDestroyBuffer(m_Device, Buffer, ALLOCATOR);
}