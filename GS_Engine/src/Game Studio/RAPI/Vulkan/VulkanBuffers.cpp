#include "Vulkan.h"

#include "VulkanBuffers.h"

#include "VulkanRenderer.h"

#include <cstring>
#include "Vk_CommandBuffer.h"

#include "Vk_Queue.h"

// BASE BUFFER

Vk_Buffer::Vk_Buffer(VkDevice _Device, void* _Data, size_t _BufferSize, VkBufferUsageFlagBits _BufferFlag, const Vk_Queue& _Queue, VkCommandPool _CP, const Vulkan_Device& _VD) : VulkanObject(_Device)
{
	//  CREATE STAGING MEMORY
	//FIND BUFFER MEMORY REQUIREMENTS
	VkMemoryRequirements MemoryRequirements;
	vkGetBufferMemoryRequirements(m_Device, Buffer, &MemoryRequirements);

	VkMemoryAllocateInfo StagingBufferMemoryAllocateInfo = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	StagingBufferMemoryAllocateInfo.allocationSize = MemoryRequirements.size;
	StagingBufferMemoryAllocateInfo.memoryTypeIndex = _VD.FindMemoryType(MemoryRequirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);

	//ALLOCATE STAGING BUFFER MEMORY
	GS_VK_CHECK(vkAllocateMemory(m_Device, &StagingBufferMemoryAllocateInfo, ALLOCATOR, &Memory), "Failed to allocate memory!")

	//CREATE STAGING BUFFER
	VkBufferCreateInfo StagingBufferCreateInfo = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
	StagingBufferCreateInfo.size = _BufferSize;
	StagingBufferCreateInfo.usage = _BufferFlag;
	StagingBufferCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	VkBuffer StagingBuffer;
	GS_VK_CHECK(vkCreateBuffer(m_Device, &StagingBufferCreateInfo, ALLOCATOR, &StagingBuffer), "Failed to allocate Buffer!")

	//COPY DATA TO STAGING BUFFER
	GS_VK_CHECK(vkBindBufferMemory(m_Device, Buffer, Memory, 0), "Failed to bind buffer memory!")//Copy Memory
	void* Data;
	GS_VK_CHECK(vkMapMemory(m_Device, Memory, 0, _BufferSize, 0, &Data), "Failed to map memory!")
	memcpy(Data, _Data, _BufferSize);
	vkUnmapMemory(m_Device, Memory);

	//  CREATE DEVICE MEMORY
	VkMemoryAllocateInfo BufferMemoryAllocateInfo = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	BufferMemoryAllocateInfo.allocationSize = MemoryRequirements.size;
	BufferMemoryAllocateInfo.memoryTypeIndex = _VD.FindMemoryType(MemoryRequirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);

	//ALLOCATE DEVICE BUFFER MEMORY
	VkDeviceMemory StagingBufferMemory;
	GS_VK_CHECK(vkAllocateMemory(m_Device, &BufferMemoryAllocateInfo, ALLOCATOR, &StagingBufferMemory), "Failed to allocate memory!")

	// CREATE BUFFER
	VkBufferCreateInfo BufferCreateInfo = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
	BufferCreateInfo.size = _BufferSize;
	BufferCreateInfo.usage = _BufferFlag;
	BufferCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	GS_VK_CHECK(vkCreateBuffer(m_Device, &BufferCreateInfo, ALLOCATOR, &Buffer), "Failed to allocate Buffer!")

	Vk_CommandBuffer CommandBuffer(m_Device, _CP);

	VkCommandBufferBeginInfo CommandBufferBeginInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
	CommandBufferBeginInfo.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;

	CommandBuffer.Begin(&CommandBufferBeginInfo);

	VkBufferCopy MemoryCopyInfo = {};
	MemoryCopyInfo.srcOffset = 0; // Optional
	MemoryCopyInfo.dstOffset = 0; // Optional
	MemoryCopyInfo.size = _BufferSize;
	vkCmdCopyBuffer(CommandBuffer.GetVkCommandBuffer(), StagingBuffer, Buffer, 1, &MemoryCopyInfo);

	CommandBuffer.End();

	VkSubmitInfo SubmitInfo = {};
	SubmitInfo.commandBufferCount = 1;
	SubmitInfo.pCommandBuffers = CommandBuffer;
	_Queue.Submit(&SubmitInfo, VK_NULL_HANDLE);

	CommandBuffer.Free(_CP);

	vkDestroyBuffer(m_Device, StagingBuffer, ALLOCATOR);
	vkFreeMemory(m_Device, StagingBufferMemory, ALLOCATOR);
}

Vk_Buffer::~Vk_Buffer()
{
	vkDestroyBuffer(m_Device, Buffer, ALLOCATOR);
	vkFreeMemory(m_Device, Memory, ALLOCATOR);
}


VkBufferUsageFlagBits BufferTypeToVkBufferUsageFlagBits(BufferType _BT)
{
	switch (_BT)
	{
	case BufferType::BUFFER_VERTEX:		return VK_BUFFER_USAGE_VERTEX_BUFFER_BIT;
	case BufferType::BUFFER_INDEX:		return VK_BUFFER_USAGE_INDEX_BUFFER_BIT;
	case BufferType::BUFFER_UNIFORM:	return VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
	default:							return VK_BUFFER_USAGE_FLAG_BITS_MAX_ENUM;
	}
}

VulkanBuffer::VulkanBuffer(VkDevice _Device, void* _Data, size_t _BufferSize, BufferType _BufferType, const Vk_Queue& _Queue, VkCommandPool _CP, const Vulkan_Device& _VD): Buffer(_Device, _Data, _BufferSize, BufferTypeToVkBufferUsageFlagBits(_BufferType), _Queue, _CP, _VD)
{
}

VulkanBuffer::~VulkanBuffer()
{
}
