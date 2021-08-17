#pragma once

#include "Vulkan.h"
#include "VulkanRenderDevice.h"

namespace GAL
{
	class VulkanQueryPool
	{
	public:
		VulkanQueryPool() = default;
		
		void Initialize(const VulkanRenderDevice* renderDevice, QueryType queryType, GTSL::uint32 queryCount) {
			VkQueryPoolCreateInfo vk_query_pool_create_info{ VK_STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO };
			vk_query_pool_create_info.queryCount = queryCount;
			vk_query_pool_create_info.queryType = ToVulkan(queryType);

			renderDevice->VkCreateQueryPool(renderDevice->GetVkDevice(), &vk_query_pool_create_info, renderDevice->GetVkAllocationCallbacks(), &queryPool);
			//setName(createInfo.RenderDevice, queryPool, VK_OBJECT_TYPE_QUERY_POOL, createInfo.Name);
		}

		void GetQueryResults(const VulkanRenderDevice* renderDevice, void* data, GTSL::uint32 size, GTSL::uint32 queryCount, GTSL::uint32 stride, bool wait) const {
			VkQueryResultFlags flags = 0;
			GTSL::SetBitAs(1, wait, flags);

			renderDevice->VkGetQueryPoolResults(renderDevice->GetVkDevice(), queryPool, 0, queryCount, size, data, stride, flags);
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyQueryPool(renderDevice->GetVkDevice(), queryPool, renderDevice->GetVkAllocationCallbacks());
			debugClear(queryPool);
		}
		
		[[nodiscard]] VkQueryPool GetVkQueryPool() const { return queryPool; }
	private:
		VkQueryPool queryPool;
	};
}
