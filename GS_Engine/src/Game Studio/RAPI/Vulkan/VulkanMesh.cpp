#include "VulkanMesh.h"

#include "VulkanRenderer.h"
#include <vulkan/vulkan_core.h>

VulkanMesh::VulkanMesh(const Vk_Device& _Device, const Vk_CommandPool& _CP, void* _VertexData, size_t _VertexDataSize, uint16* _IndexData, uint16 _IndexCount) :
	VertexBuffer(_Device, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, _VertexDataSize),
	VBMemory(_Device),
	IndexBuffer(_Device, VK_BUFFER_USAGE_INDEX_BUFFER_BIT, _VertexDataSize),
	IBMemory(_Device)
{
	Vk_Buffer StagingVB(_Device, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, _VertexDataSize);
	Vk_Buffer StagingIB(_Device, VK_BUFFER_USAGE_INDEX_BUFFER_BIT, _IndexCount * sizeof(uint16));

	Vk_Memory StagingVBMemory(_Device);
	Vk_Memory StagingIBMemory(_Device);

	StagingVBMemory.CopyToMappedMemory(_VertexData, _VertexDataSize);
	StagingIBMemory.CopyToMappedMemory(_IndexData, _IndexCount * sizeof(uint16));


	VkMemoryRequirements VBMemoryRequirements;
	vkGetBufferMemoryRequirements(_Device, VertexBuffer, &VBMemoryRequirements);
	VkMemoryRequirements IBMemoryRequirements;
	vkGetBufferMemoryRequirements(_Device, IndexBuffer, &IBMemoryRequirements);
	VBMemory.AllocateDeviceMemory(&VBMemoryRequirements);
	IBMemory.AllocateDeviceMemory(&IBMemoryRequirements);

	VBMemory.BindBufferMemory(VertexBuffer);
	VBMemory.BindBufferMemory(IndexBuffer);

	VBMemory.CopyToDevice(StagingVB, VertexBuffer, _CP, _Device.GetTransferQueue(), _VertexDataSize);
	VBMemory.CopyToDevice(StagingIB, IndexBuffer, _CP, _Device.GetTransferQueue(), _IndexCount * sizeof(uint16));
}
