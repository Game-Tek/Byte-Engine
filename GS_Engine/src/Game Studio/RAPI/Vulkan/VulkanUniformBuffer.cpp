#include "VulkanUniformBuffer.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "RAPI/Vulkan/Native/VKDevice.h"

VKBufferCreator VulkanUniformBuffer::CreateBuffer(VKDevice* _Device, const UniformBufferCreateInfo& _BCI)
{
	VkBufferCreateInfo BufferCreateInfo = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
	BufferCreateInfo.size = _BCI.Size;
	BufferCreateInfo.usage = VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
	BufferCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	return VKBufferCreator(_Device, &BufferCreateInfo);
}

VKMemoryCreator VulkanUniformBuffer::CreateMemory(VKDevice* _Device)
{
	auto MemReqs = Buffer.GetMemoryRequirements();

	VkMemoryAllocateInfo MemoryAllocateInfo = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
	MemoryAllocateInfo.allocationSize = MemReqs.size;
	MemoryAllocateInfo.memoryTypeIndex = _Device->FindMemoryType(MemReqs.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);

	return VKMemoryCreator(_Device, &MemoryAllocateInfo);
}

VulkanUniformBuffer::VulkanUniformBuffer(VKDevice* _Device, const UniformBufferCreateInfo& _BCI) : Buffer(CreateBuffer(_Device, _BCI)), Memory(CreateMemory(_Device))
{
	Memory.BindBufferMemory(Buffer);
	MappedMemoryPointer = Memory.MapMemory(0, _BCI.Size);
	Memory.CopyToMappedMemory(_BCI.Data, MappedMemoryPointer, _BCI.Size);
}

VulkanUniformBuffer::~VulkanUniformBuffer()
{
	Memory.UnmapMemory();
}

void VulkanUniformBuffer::UpdateBuffer(const UniformBufferUpdateInfo& _BUI) const
{
	Memory.CopyToMappedMemory(_BUI.Data, MappedMemoryPointer, _BUI.Size);
}
