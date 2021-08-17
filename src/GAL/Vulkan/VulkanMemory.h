#pragma once

#include "GAL/Memory.h"

#include "Vulkan.h"
#include "VulkanRenderDevice.h"

namespace GAL
{
	class VulkanDeviceMemory final : public DeviceMemory
	{
	public:		
		VulkanDeviceMemory() = default;
		
		bool Initialize(const VulkanRenderDevice* renderDevice, AllocationFlag flags, GTSL::uint32 size, MemoryType memoryType) {
			VkMemoryAllocateInfo vkMemoryAllocateInfo{ VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };

			VkMemoryAllocateFlagsInfo vkMemoryAllocateFlagsInfo{ VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO };
			vkMemoryAllocateFlagsInfo.flags = ToVulkan(flags);

			vkMemoryAllocateInfo.pNext = &vkMemoryAllocateFlagsInfo;
			vkMemoryAllocateInfo.allocationSize = size;
			vkMemoryAllocateInfo.memoryTypeIndex = renderDevice->GetMemoryTypeIndex(memoryType);

			return renderDevice->VkAllocateMemory(renderDevice->GetVkDevice(), &vkMemoryAllocateInfo, renderDevice->GetVkAllocationCallbacks(), &deviceMemory) == VK_SUCCESS;
			//setName(info.RenderDevice, deviceMemory, VK_OBJECT_TYPE_DEVICE_MEMORY, info.Name);
		}
		
		~VulkanDeviceMemory() = default;

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkFreeMemory(renderDevice->GetVkDevice(), deviceMemory, renderDevice->GetVkAllocationCallbacks());
			debugClear(deviceMemory);
		}
		
		[[nodiscard]] VkDeviceMemory GetVkDeviceMemory() const { return deviceMemory; }

		[[nodiscard]] void* Map(const VulkanRenderDevice* renderDevice, const GTSL::uint32 size, const GTSL::uint32 offset) const {
			void* data = nullptr;
			renderDevice->VkMapMemory(renderDevice->GetVkDevice(), deviceMemory, offset, size, 0, &data);
			return data;
		}
		
		void Unmap(const VulkanRenderDevice* renderDevice) const {
			renderDevice->VkUnmapMemory(renderDevice->GetVkDevice(), deviceMemory);
		}
		
	private:
		VkDeviceMemory deviceMemory = nullptr;
	};
}
