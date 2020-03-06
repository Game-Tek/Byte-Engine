#include "VulkanRenderMesh.h"

#include "VulkanRenderDevice.h"
#include <RAPI\Vulkan\VulkanCommandBuffer.h>

VulkanRenderMesh::VulkanRenderMesh(VulkanRenderDevice* vulkanRenderDevice, const RAPI::RenderMesh::RenderMeshCreateInfo& renderMeshCreateInfo)
{
	size_t vertex_buffer_size = renderMeshCreateInfo.VertexCount * renderMeshCreateInfo.VertexLayout->GetSize();
	size_t index_buffer_size = renderMeshCreateInfo.IndexCount * sizeof(uint16);
	size_t buffer_size = vertex_buffer_size + index_buffer_size;

	VkBuffer staging_buffer = nullptr;
	VkDeviceMemory staging_memory = nullptr;

	VkBufferCreateInfo vk_buffer_create_info{ VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
	vk_buffer_create_info.size = buffer_size;
	vk_buffer_create_info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
	vk_buffer_create_info.usage = VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_INDEX_BUFFER_BIT;

	vkCreateBuffer(vulkanRenderDevice->GetVkDevice(), &vk_buffer_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &staging_buffer);
	vkCreateBuffer(vulkanRenderDevice->GetVkDevice(), &vk_buffer_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &buffer);

	VkMemoryRequirements vk_memory_requirements;
	vkGetBufferMemoryRequirements(vulkanRenderDevice->GetVkDevice(), buffer, &vk_memory_requirements);

	VkMemoryAllocateInfo vk_staging_memory_allocate_info = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	vk_staging_memory_allocate_info.allocationSize = vk_memory_requirements.size;
	vk_staging_memory_allocate_info.memoryTypeIndex = vulkanRenderDevice->FindMemoryType(vk_memory_requirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
	vkAllocateMemory(vulkanRenderDevice->GetVkDevice(), &vk_staging_memory_allocate_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &staging_memory);

	VkMemoryAllocateInfo vk_memory_allocate_info = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	vk_staging_memory_allocate_info.allocationSize = vk_memory_requirements.size;
	vk_staging_memory_allocate_info.memoryTypeIndex = vulkanRenderDevice->FindMemoryType(vk_memory_requirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
	vkAllocateMemory(vulkanRenderDevice->GetVkDevice(), &vk_memory_allocate_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &memory);

	vkBindBufferMemory(vulkanRenderDevice->GetVkDevice(), staging_buffer, staging_memory, 0);
	vkBindBufferMemory(vulkanRenderDevice->GetVkDevice(), buffer, memory, 0);

	byte* mapped_staging_buffer_memory = nullptr;
	vkMapMemory(vulkanRenderDevice->GetVkDevice(), staging_memory, 0, buffer_size, 0, (void**)&mapped_staging_buffer_memory);

	memcpy(mapped_staging_buffer_memory, renderMeshCreateInfo.VertexData, vertex_buffer_size);
	memcpy(mapped_staging_buffer_memory + vertex_buffer_size, renderMeshCreateInfo.IndexData, index_buffer_size);

	vkUnmapMemory(vulkanRenderDevice->GetVkDevice(), staging_memory);

	VkBufferCopy vk_region;
	vk_region.srcOffset = 0;
	vk_region.dstOffset = 0;
	vk_region.size = buffer_size;
	vkCmdCopyBuffer(static_cast<VulkanCommandBuffer*>(renderMeshCreateInfo.CommandBuffer)->GetVkCommandBuffer(), staging_buffer, buffer, 1, &vk_region);

	vkDestroyBuffer(vulkanRenderDevice->GetVkDevice(), staging_buffer, vulkanRenderDevice->GetVkAllocationCallbacks());
	vkFreeMemory(vulkanRenderDevice->GetVkDevice(), staging_memory, vulkanRenderDevice->GetVkAllocationCallbacks());
}

void VulkanRenderMesh::Destroy(RenderDevice* renderDevice)
{
	auto vk_render_device = static_cast<VulkanRenderDevice*>(renderDevice);
	vkDestroyBuffer(vk_render_device->GetVkDevice(), buffer, vk_render_device->GetVkAllocationCallbacks());
	vkFreeMemory(vk_render_device->GetVkDevice(), memory, vk_render_device->GetVkAllocationCallbacks());
}
