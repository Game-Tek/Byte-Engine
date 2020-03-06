#include "VulkanUniformBuffer.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VulkanRenderDevice.h"

VulkanUniformBuffer::VulkanUniformBuffer(VulkanRenderDevice* vulkanRenderDevice, const UniformBufferCreateInfo& _BCI)
{
	VkBufferCreateInfo BufferCreateInfo = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
	BufferCreateInfo.size = _BCI.Size;
	BufferCreateInfo.usage = VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
	BufferCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	VkMemoryRequirements vk_memory_requirements;

	vkGetBufferMemoryRequirements(vulkanRenderDevice->GetVkDevice(), buffer, &vk_memory_requirements);

	VkMemoryAllocateInfo vk_memory_allocate_info = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	vk_memory_allocate_info.allocationSize = vk_memory_requirements.size;
	vk_memory_allocate_info.memoryTypeIndex = vulkanRenderDevice->findMemorytype(vk_memory_requirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);

	vkBindBufferMemory(vulkanRenderDevice->GetVkDevice(), buffer, memory, 0/*offset*/);


	vkMapMemory(vulkanRenderDevice->GetVkDevice(), memory, 0/*offset*/, vk_memory_requirements.size, 0/*flags*/, (void**)&mappedMemoryPointer);
}

VulkanUniformBuffer::~VulkanUniformBuffer()
{
	Memory.UnmapMemory();
}

void VulkanUniformBuffer::UpdateBuffer(const UniformBufferUpdateInfo& uniformBufferUpdateInfo) const
{
	Memory.CopyToMappedMemory(uniformBufferUpdateInfo.Data, mappedMemoryPointer + uniformBufferUpdateInfo.Offset, uniformBufferUpdateInfo.Size);
}
