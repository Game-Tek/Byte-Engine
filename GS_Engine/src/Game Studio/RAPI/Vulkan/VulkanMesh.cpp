#include "VulkanMesh.h"

#include "VulkanRenderer.h"
#include <vulkan/vulkan_core.h>

VulkanMesh::VulkanMesh(const Vk_Device& _Device) : 
	VertexBuffer(_Device, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, _Size),
	IndexBuffer(_Device, VK_BUFFER_USAGE_INDEX_BUFFER_BIT, _Size),
	VBMemory(_Device, VertexBuffer),
	IBMemory(_Device, IndexBuffer)
{
	Vk_Buffer StagingVB(_Device, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, _VertexSize);
	Vk_Buffer StagingIB(_Device, VK_BUFFER_USAGE_INDEX_BUFFER_BIT, _IndexSize);

	Vk_Memory StagingVBMemory(_Device, StagingVB);
	Vk_Memory StagingIBMemory(_Device, StagingIB);

	StagingVBMemory.CopyToMappedMemory(_VertexData, Size);
	StagingIBMemory.CopyToMappedMemory(IndexData, _IndexSize);

	VBMemory.AllocateDeviceMemory(_Device, VBMemoryRequirements);
	IBMemory.AllocateDeviceMemory(_Device, IBMemoryRequirements);

	VBMemory.CopyToDevice(StagingVB, VertexBuffer, _CP, _Device.GetTransferQueue(), _VertexSize);
	VBMemory.CopyToDevice(StagingIB, IndexBuffer, _CP, _Device.GetTransferQueue(), _IndexSize);
}
