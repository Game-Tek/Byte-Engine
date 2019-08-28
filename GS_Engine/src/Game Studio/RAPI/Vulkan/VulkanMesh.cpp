#include "VulkanMesh.h"

#include "VulkanRenderer.h"
#include <vulkan/vulkan_core.h>

VulkanMesh::VulkanMesh(const Vk_Device& _Device, const Vk_CommandPool& _CP, void* _VertexData, size_t _VertexDataSize, uint16* _IndexData, uint16 _IndexCount) :
	VertexBuffer(_Device, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT, _VertexDataSize),
	VBMemory(_Device),
	IndexBuffer(_Device, VK_BUFFER_USAGE_INDEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT, _IndexCount * sizeof(uint16)),
	IBMemory(_Device)
{
	//VBMemory.AllocateDeviceMemory(VertexBuffer.GetRequirements(), VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT);
	//IBMemory.AllocateDeviceMemory(IndexBuffer.GetRequirements(), VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT);
	//
	//VBMemory.BindBufferMemory(VertexBuffer);
	//IBMemory.BindBufferMemory(IndexBuffer);
	//
	//VBMemory.CopyToMappedMemory(_VertexData, _VertexDataSize);
	//IBMemory.CopyToMappedMemory(_IndexData, _IndexCount * sizeof(uint16));

	Vk_Buffer StagingVB(_Device, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_SRC_BIT, _VertexDataSize);
	Vk_Buffer StagingIB(_Device, VK_BUFFER_USAGE_INDEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_SRC_BIT, _IndexCount * sizeof(uint16));
	
	Vk_Memory StagingVBMemory(_Device);
	Vk_Memory StagingIBMemory(_Device);
	
	StagingVBMemory.AllocateDeviceMemory(StagingVB.GetRequirements(), VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT);
	StagingIBMemory.AllocateDeviceMemory(StagingIB.GetRequirements(), VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT);
	
	StagingVBMemory.BindBufferMemory(StagingVB);
	StagingIBMemory.BindBufferMemory(StagingIB);
	
	StagingVBMemory.CopyToMappedMemory(_VertexData, _VertexDataSize);
	StagingIBMemory.CopyToMappedMemory(_IndexData, _IndexCount * sizeof(uint16));
	
	VBMemory.AllocateDeviceMemory(VertexBuffer.GetRequirements(), VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
	IBMemory.AllocateDeviceMemory(IndexBuffer.GetRequirements(), VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
	
	VBMemory.BindBufferMemory(VertexBuffer);
	IBMemory.BindBufferMemory(IndexBuffer);
	
	VBMemory.CopyToDevice(StagingVB, VertexBuffer, _CP, _Device.GetTransferQueue(), _VertexDataSize);
	IBMemory.CopyToDevice(StagingIB, IndexBuffer, _CP, _Device.GetTransferQueue(), _IndexCount * sizeof(uint16));
}
