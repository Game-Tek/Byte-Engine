#pragma once

#include "Vulkan.h"
#include "VulkanRenderDevice.h"

namespace GAL
{
	class VulkanRenderDevice;

	class VulkanAsyncHostOperation
	{
	public:
		VulkanAsyncHostOperation() = default;

		void Initialize(const VulkanRenderDevice* renderDevice) {
			renderDevice->vkCreateDeferredOperationKHR(renderDevice->GetVkDevice(), renderDevice->GetVkAllocationCallbacks(), &deferredOperation);
		}

		GTSL::uint32 GetMaxConcurrency(const VulkanRenderDevice* renderDevice) {
			return renderDevice->vkGetDeferredOperationMaxConcurrencyKHR(renderDevice->GetVkDevice(), deferredOperation);
		}
		
		bool GetResult(const VulkanRenderDevice* renderDevice) {
			return renderDevice->vkGetDeferredOperationResultKHR(renderDevice->GetVkDevice(), deferredOperation) == VK_SUCCESS;
		}

		enum class JoinResult {
			DONE, PENDING, WAITING
		};
		JoinResult Join(const VulkanRenderDevice* renderDevice) {
			switch (renderDevice->vkDeferredOperationJoinKHR(renderDevice->GetVkDevice(), deferredOperation))
			{
			case VK_SUCCESS: return JoinResult::DONE;
			case VK_THREAD_DONE_KHR: return JoinResult::PENDING;
			case VK_THREAD_IDLE_KHR: return JoinResult::WAITING;
			}
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->vkDestroyDeferredOperationKHR(renderDevice->GetVkDevice(), deferredOperation, renderDevice->GetVkAllocationCallbacks());
		}

		[[nodiscard]] VkDeferredOperationKHR GetVkDeferredHostOperationKHR() const { return deferredOperation; }
		
	private:
		VkDeferredOperationKHR deferredOperation;
	};
}
