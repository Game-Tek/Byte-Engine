#include "VulkanMesh.h"

#include "VulkanRenderer.h"
#include <vulkan/vulkan_core.h>

VKBufferCreator VulkanMesh::CreateVKBufferCreator(VKDevice* _Device, unsigned _BufferUsage, size_t _BufferSize)
{
	VkBufferCreateInfo BufferCreateInfo = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
	BufferCreateInfo.size = _BufferSize;
	BufferCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
	BufferCreateInfo.usage = _BufferUsage;

	return VKBufferCreator(_Device, &BufferCreateInfo);
}

VKMemoryCreator VulkanMesh::CreateVKMemoryCreator(VKDevice* _Device, VkMemoryRequirements _MemReqs,	unsigned _MemoryProps)
{
	VkMemoryAllocateInfo MemoryAllocateInfo = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	MemoryAllocateInfo.allocationSize = _MemReqs.size;
	MemoryAllocateInfo.memoryTypeIndex = _Device->FindMemoryType(_MemReqs.memoryTypeBits, _MemoryProps);

	return VKMemoryCreator(_Device, &MemoryAllocateInfo);
}

VulkanMesh::VulkanMesh(VKDevice* _Device, const VKCommandPool& _CP, void* _VertexData, size_t _VertexDataSize, uint16* _IndexData, uint16 _IndexCount) :
	VertexBuffer(CreateVKBufferCreator(_Device, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT, _VertexDataSize)),
	VBMemory(CreateVKMemoryCreator(_Device, VertexBuffer.GetMemoryRequirements(), VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)),
	IndexBuffer(CreateVKBufferCreator(_Device, VK_BUFFER_USAGE_INDEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT, _IndexCount * sizeof(uint16))),
	IBMemory(CreateVKMemoryCreator(_Device, IndexBuffer.GetMemoryRequirements(), VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT))
{
	VBMemory.BindBufferMemory(VertexBuffer);
	IBMemory.BindBufferMemory(IndexBuffer);

	VKBuffer StagingVB(CreateVKBufferCreator(_Device, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_SRC_BIT, _VertexDataSize));
	VKBuffer StagingIB(CreateVKBufferCreator(_Device, VK_BUFFER_USAGE_INDEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_SRC_BIT, _IndexCount * sizeof(uint16)));
	
	VKMemory StagingVBMemory(CreateVKMemoryCreator(_Device, StagingVB.GetMemoryRequirements(), VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT));
	VKMemory StagingIBMemory(CreateVKMemoryCreator(_Device, StagingIB.GetMemoryRequirements(), VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT));
	
	StagingVBMemory.BindBufferMemory(StagingVB);
	StagingIBMemory.BindBufferMemory(StagingIB);
	
	StagingVBMemory.SingleCopyToMappedMemory(_VertexData, _VertexDataSize);
	StagingIBMemory.SingleCopyToMappedMemory(_IndexData, _IndexCount * sizeof(uint16));
	
	
	VBMemory.CopyToDevice(StagingVB, VertexBuffer, _CP, _Device->GetTransferQueue(), _VertexDataSize);
	IBMemory.CopyToDevice(StagingIB, IndexBuffer, _CP, _Device->GetTransferQueue(), _IndexCount * sizeof(uint16));
}
