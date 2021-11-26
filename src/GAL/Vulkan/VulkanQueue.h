#pragma once
#include "VulkanCommandList.h"
#include "GTSL/Core.h"

#include "VulkanRenderDevice.h"
#include "VulkanSynchronization.h"
#include "GAL/Queue.h"

namespace GAL {
	class VulkanQueue final : public Queue
	{
	public:
		VulkanQueue() = default;
		~VulkanQueue() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, const VulkanRenderDevice::QueueKey queue_key) {
			queueKey = queue_key;
			renderDevice->getDeviceProcAddr<PFN_vkGetDeviceQueue>(u8"vkGetDeviceQueue")(renderDevice->GetVkDevice(), queueKey.Family, queueKey.Queue, &queue);
		}

		void Wait(const VulkanRenderDevice* renderDevice) const {
			renderDevice->VkQueueWaitIdle(queue);
		}

		bool Submit(const VulkanRenderDevice* renderDevice, const GTSL::Range<const WorkUnit*> submitInfos, VulkanFence& fence) {
			VkSubmitInfo vkSubmitInfo{ VK_STRUCTURE_TYPE_SUBMIT_INFO };
			GTSL::StaticVector<VkCommandBuffer, 16> vkCommandBuffers;
			GTSL::StaticVector<VkSemaphore, 16> signalSemaphores; GTSL::StaticVector<VkSemaphore, 16> waitSemaphores;
			GTSL::StaticVector<GTSL::uint64, 16> signalValues; GTSL::StaticVector<GTSL::uint64, 16> waitValues;
			GTSL::StaticVector<VkPipelineStageFlags, 16> waitPipelineStages;

			for(auto& s : submitInfos) {
				if (s.CommandBuffer) {
					vkCommandBuffers.EmplaceBack(static_cast<const VulkanCommandList*>(s.CommandBuffer)->GetVkCommandBuffer());
				}
				
				if (s.SignalSemaphore) {
					signalSemaphores.EmplaceBack(static_cast<const VulkanSemaphore*>(s.SignalSemaphore)->GetVkSemaphore());
					static_cast<VulkanSemaphore*>(s.SignalSemaphore)->Signal();
				}

				if (s.WaitSemaphore) {
					waitSemaphores.EmplaceBack(static_cast<const VulkanSemaphore*>(s.WaitSemaphore)->GetVkSemaphore());
					waitPipelineStages.EmplaceBack(ToVulkan(s.WaitPipelineStage));
					static_cast<VulkanSemaphore*>(s.WaitSemaphore)->Unsignal();
				}				
			}
			
			vkSubmitInfo.commandBufferCount = vkCommandBuffers.GetLength();
			vkSubmitInfo.pCommandBuffers = vkCommandBuffers.begin();
			vkSubmitInfo.waitSemaphoreCount = waitSemaphores.GetLength();
			vkSubmitInfo.pWaitSemaphores = waitSemaphores.begin();
			vkSubmitInfo.signalSemaphoreCount = signalSemaphores.GetLength();
			vkSubmitInfo.pSignalSemaphores = signalSemaphores.begin();
			vkSubmitInfo.pWaitDstStageMask = waitPipelineStages.begin();

			fence.Signal();
			
			return renderDevice->VkQueueSubmit(queue, 1, &vkSubmitInfo, fence.GetVkFence()) == VK_SUCCESS;
		}

		[[nodiscard]] VkQueue GetVkQueue() const { return queue; }
		[[nodiscard]] VulkanRenderDevice::QueueKey GetQueueKey() const { return queueKey; }

	private:
		VkQueue queue = nullptr;
		VulkanRenderDevice::QueueKey queueKey;
	};
}
