#include "Vk_Memory.h"

#include <cstring>

#include "Vk_Device.h"
#include "Vk_Buffer.h"
#include "Vk_CommandPool.h"
#include "Vk_CommandBuffer.h"
#include "Vk_Queue.h"
#include "Vk_Image.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "Vk_Fence.h"

Vk_Memory::Vk_Memory(const Vk_Device& _Device) : VulkanObject(_Device)
{
}

void Vk_Memory::CopyToDevice(const Vk_Buffer& _SrcBuffer, const Vk_Buffer& _DstBuffer, const Vk_CommandPool& _CP, const Vk_Queue& _Queue, size_t _Size) const
{
	Vk_CommandBuffer CommandBuffer(m_Device, _CP);

	VkCommandBufferBeginInfo CommandBufferBeginInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
	CommandBufferBeginInfo.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;

	CommandBuffer.Begin(&CommandBufferBeginInfo);

	VkBufferCopy MemoryCopyInfo = {};
	MemoryCopyInfo.size = _Size;
	vkCmdCopyBuffer(CommandBuffer, _SrcBuffer, _DstBuffer, 1, &MemoryCopyInfo);

	CommandBuffer.End();

	VkSubmitInfo SubmitInfo = { VK_STRUCTURE_TYPE_SUBMIT_INFO };
	SubmitInfo.commandBufferCount = 1;
	SubmitInfo.pCommandBuffers = CommandBuffer;

	_Queue.Submit(&SubmitInfo, VK_NULL_HANDLE);
	_Queue.Wait();

	CommandBuffer.Free(_CP);
}

void Vk_Memory::BindBufferMemory(const Vk_Buffer& _Buffer) const
{
	vkBindBufferMemory(m_Device, _Buffer, Memory, 0);
}

void Vk_Memory::BindImageMemory(const Vk_Image& _Image) const
{
	vkBindImageMemory(m_Device, _Image, Memory, 0);
}

Vk_Memory::~Vk_Memory()
{
	vkFreeMemory(m_Device, Memory, ALLOCATOR);
}

void Vk_Memory::AllocateDeviceMemory(const VkMemoryRequirements& _MR, unsigned _MemProps)
{
	VkMemoryAllocateInfo MemoryAllocateInfo = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	MemoryAllocateInfo.allocationSize = _MR.size;
	MemoryAllocateInfo.memoryTypeIndex = m_Device.FindMemoryType(_MR.memoryTypeBits, _MemProps);

	GS_VK_CHECK(vkAllocateMemory(m_Device, &MemoryAllocateInfo, ALLOCATOR, &Memory), "Failed to allocate memory!")
}

void* Vk_Memory::CopyToMappedMemory(void* _Data, size_t _Size)
{
	void* data = nullptr;
	vkMapMemory(m_Device, Memory, 0, _Size, 0, &data);
	memcpy(data, _Data, _Size);
	vkUnmapMemory(m_Device, Memory);
	return data;
}