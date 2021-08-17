#pragma once

#include "GAL/Buffer.h"

#include "Vulkan.h"
#include "VulkanMemory.h"
#include "VulkanRenderDevice.h"

namespace GAL
{
	class VulkanBuffer final : public Buffer
	{
	public:
		VulkanBuffer() = default;
		~VulkanBuffer() = default;
		
		void GetMemoryRequirements(const VulkanRenderDevice* renderDevice, GTSL::uint32 size, BufferUse bufferUses, MemoryRequirements* memoryRequirements)
		{
			VkBufferCreateInfo vkBufferCreateInfo{ VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
			vkBufferCreateInfo.size = size;
			vkBufferCreateInfo.usage = ToVulkan(bufferUses);
			vkBufferCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

			renderDevice->VkCreateBuffer(renderDevice->GetVkDevice(), &vkBufferCreateInfo, renderDevice->GetVkAllocationCallbacks(), &buffer);

			VkMemoryRequirements vkMemoryRequirements;
			renderDevice->VkGetBufferMemoryRequirements(renderDevice->GetVkDevice(), buffer, &vkMemoryRequirements);
			memoryRequirements->Size = static_cast<GTSL::uint32>(vkMemoryRequirements.size);
			memoryRequirements->Alignment = static_cast<GTSL::uint32>(vkMemoryRequirements.alignment);
			memoryRequirements->MemoryTypes = vkMemoryRequirements.memoryTypeBits;
		}
		
		void Initialize(const VulkanRenderDevice* renderDevice, const MemoryRequirements& memoryRequirements, VulkanDeviceMemory memory, GTSL::uint32 offset) {
			//SET_NAME(buffer, VK_OBJECT_TYPE_BUFFER, info);
			renderDevice->VkBindBufferMemory(renderDevice->GetVkDevice(), buffer, static_cast<VkDeviceMemory>(memory.GetVkDeviceMemory()), offset);
		}
		
		[[nodiscard]] DeviceAddress GetAddress(const VulkanRenderDevice* renderDevice) const {
			VkBufferDeviceAddressInfo vkBufferDeviceAddressInfo{ VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO };
			vkBufferDeviceAddressInfo.buffer = buffer;
			return DeviceAddress(renderDevice->VkGetBufferDeviceAddress(renderDevice->GetVkDevice(), &vkBufferDeviceAddressInfo));
		}

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyBuffer(renderDevice->GetVkDevice(), buffer, renderDevice->GetVkAllocationCallbacks());
			debugClear(buffer);
		}
		
		[[nodiscard]] VkBuffer GetVkBuffer() const { return buffer; }
		
	private:
		VkBuffer buffer = nullptr;
	};
}
