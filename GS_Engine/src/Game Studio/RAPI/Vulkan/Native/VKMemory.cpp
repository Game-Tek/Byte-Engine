#include "VKMemory.h"

#include <cstring>

#include "VKDevice.h"
#include "VKBuffer.h"
#include "VKCommandPool.h"
#include "VKCommandBuffer.h"
#include "vkQueue.h"
#include "VKImage.h"

#include "RAPI/Vulkan/Vulkan.h"

VKMemoryCreator::
VKMemoryCreator(VKDevice* _Device, const VkMemoryAllocateInfo* _VkMAI) : VKObjectCreator<VkDeviceMemory>(_Device)
{
	GS_VK_CHECK(vkAllocateMemory(m_Device->GetVkDevice(), _VkMAI, ALLOCATOR, &Handle), "Failed to allocate memory!")
}

void VKMemory::CopyToDevice(const VKBuffer& _SrcBuffer, const VKBuffer& _DstBuffer, const VKCommandPool& _CP,
                            const vkQueue& _Queue, size_t _Size) const
{
	VKCommandBuffer CommandBuffer(_CP.CreateCommandBuffer());

	VkCommandBufferBeginInfo CommandBufferBeginInfo = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
	CommandBufferBeginInfo.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;

	CommandBuffer.Begin(&CommandBufferBeginInfo);

	VkBufferCopy MemoryCopyInfo = {};
	MemoryCopyInfo.size = _Size;
	vkCmdCopyBuffer(CommandBuffer, _SrcBuffer.GetHandle(), _DstBuffer.GetHandle(), 1, &MemoryCopyInfo);

	CommandBuffer.End();

	VkCommandBuffer pCommandBuffer = CommandBuffer.GetHandle();

	VkSubmitInfo SubmitInfo = {VK_STRUCTURE_TYPE_SUBMIT_INFO};
	SubmitInfo.commandBufferCount = 1;
	SubmitInfo.pCommandBuffers = &pCommandBuffer;

	_Queue.Submit(&SubmitInfo, VK_NULL_HANDLE);
	_Queue.Wait();

	CommandBuffer.Free(_CP);
}

void VKMemory::BindBufferMemory(const VKBuffer& _Buffer) const
{
	vkBindBufferMemory(m_Device->GetVkDevice(), _Buffer.GetHandle(), Handle, 0);
}

void VKMemory::BindImageMemory(const VKImage& _Image) const
{
	vkBindImageMemory(m_Device->GetVkDevice(), _Image.GetHandle(), Handle, 0);
}

VKMemory::~VKMemory()
{
	vkFreeMemory(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}

void* VKMemory::SingleCopyToMappedMemory(void* _Data, size_t _Size) const
{
	void* data = nullptr;
	vkMapMemory(m_Device->GetVkDevice(), Handle, 0, _Size, 0, &data);
	memcpy(data, _Data, _Size);
	vkUnmapMemory(m_Device->GetVkDevice(), Handle);
	return data;
}

void* VKMemory::MapMemory(size_t _Offset, size_t _Size) const
{
	void* data = nullptr;
	vkMapMemory(m_Device->GetVkDevice(), Handle, _Offset, _Size, 0, &data);
	return data;
}

void VKMemory::CopyToMappedMemory(void* _Src, void* _Dst, size_t _Size) const
{
	memcpy(_Dst, _Src, _Size);
}

void VKMemory::UnmapMemory() const
{
	vkUnmapMemory(m_Device->GetVkDevice(), Handle);
}
