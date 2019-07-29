#include "Vk_Buffer.h"

#include "RAPI/Vulkan/Vulkan.h"

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