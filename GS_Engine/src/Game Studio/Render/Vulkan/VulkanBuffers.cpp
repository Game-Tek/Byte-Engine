#include "VulkanBuffers.h"

#include "Vulkan.h"

#include "..\Renderer.h"
#include "VulkanRenderer.h"

// BASE BUFFER

VulkanBuffer::VulkanBuffer(VkDevice _Device, const Vulkan_Device& _VKD, void* _Data, size_t _BufferSize, VkBufferUsageFlagBits _BufferFlag) : VulkanObject(_Device)
{
	//Create Buffer
	VkBufferCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
	CreateInfo.size = _BufferSize;
	CreateInfo.usage = _BufferFlag;
	CreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	GS_VK_CHECK(vkCreateBuffer(m_Device, &CreateInfo, ALLOCATOR, &Buffer), "Failed to allocate Buffer!")

	//Allocate memory
	VkMemoryRequirements MemoryRequirements;
	vkGetBufferMemoryRequirements(m_Device, Buffer, &MemoryRequirements);

	VkMemoryAllocateInfo AllocateInfo = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	AllocateInfo.allocationSize = MemoryRequirements.size;
	AllocateInfo.memoryTypeIndex = _VKD.FindMemoryType(MemoryRequirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);

	GS_VK_CHECK(vkAllocateMemory(m_Device, &AllocateInfo, ALLOCATOR, &Memory), "Failed to allocate memory!")

	GS_VK_CHECK(vkBindBufferMemory(m_Device, Buffer, Memory, 0), "Failed to bind buffer memory!")//Copy Memory
	void* Data;
	GS_VK_CHECK(vkMapMemory(m_Device, Memory, 0, _BufferSize, 0, &Data), "Failed to map memory!")
	memcpy(Data, _Data, _BufferSize);
	vkUnmapMemory(m_Device, Memory);
}

VulkanBuffer::~VulkanBuffer()
{
	vkDestroyBuffer(m_Device, Buffer, ALLOCATOR);
	vkFreeMemory(m_Device, Memory, ALLOCATOR);
}