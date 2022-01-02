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
			VkResult submitResult;

			//{
			//	GTSL::StaticVector<VkSubmitInfo2KHR, 8> vkSubmitInfos;
			//
			//	GTSL::StaticVector<GTSL::StaticVector<VkCommandBufferSubmitInfoKHR, 16>, 4> vkCommandBuffers;
			//	GTSL::StaticVector<GTSL::StaticVector<VkSemaphoreSubmitInfoKHR, 16>, 4> signalSemaphores, waitSemaphores;
			//
			//	for (auto& si : submitInfos) {
			//		auto& a = vkSubmitInfos.EmplaceBack(VK_STRUCTURE_TYPE_SUBMIT_INFO_2_KHR);
			//		auto& wucb = vkCommandBuffers.EmplaceBack();
			//		auto& wuss = signalSemaphores.EmplaceBack(); auto& wuws = waitSemaphores.EmplaceBack();
			//
			//		for (auto cbi : si.CommandLists) {
			//			auto& cb = wucb.EmplaceBack(VK_STRUCTURE_TYPE_COMMAND_BUFFER_SUBMIT_INFO_KHR);
			//			cb.commandBuffer = static_cast<const VulkanCommandList*>(cbi)->GetVkCommandBuffer();
			//		}
			//
			//		for (auto& cbi : si.SignalSemaphores) {
			//			auto& s = wuss.EmplaceBack(VK_STRUCTURE_TYPE_SEMAPHORE_SUBMIT_INFO_KHR);
			//			s.semaphore = static_cast<const VulkanSemaphore*>(cbi.Semaphore)->GetVkSemaphore();
			//			s.stageMask = ToVulkan(cbi.PipelineStage);
			//			static_cast<VulkanSemaphore*>(cbi.Semaphore)->Signal();
			//		}
			//
			//		for (auto& cbi : si.WaitSemaphores) {
			//			auto& s = wuws.EmplaceBack(VK_STRUCTURE_TYPE_SEMAPHORE_SUBMIT_INFO_KHR);
			//			s.semaphore = static_cast<const VulkanSemaphore*>(cbi.Semaphore)->GetVkSemaphore();
			//			s.stageMask = ToVulkan(cbi.PipelineStage);
			//			static_cast<VulkanSemaphore*>(cbi.Semaphore)->Unsignal();
			//		}
			//
			//		a.commandBufferInfoCount = wucb.GetLength();
			//		a.pCommandBufferInfos = wucb.begin();
			//		a.waitSemaphoreInfoCount = wuws.GetLength();
			//		a.pWaitSemaphoreInfos = wuws.begin();
			//		a.signalSemaphoreInfoCount = wuss.GetLength();
			//		a.pSignalSemaphoreInfos = wuss.begin();
			//	}
			//
			//	submitResult = renderDevice->VkQueueSubmit2(queue, vkSubmitInfos.GetLength(), vkSubmitInfos.GetData(), fence.GetVkFence());
			//}

			{
				GTSL::StaticVector<VkSubmitInfo, 8> vkSubmitInfos;

				GTSL::StaticVector<GTSL::StaticVector<VkCommandBuffer, 16>, 4> vkCommandBuffers;
				GTSL::StaticVector<GTSL::StaticVector<VkSemaphore, 16>, 4> signalSemaphores, waitSemaphores;
				GTSL::StaticVector<GTSL::StaticVector<VkPipelineStageFlags, 16>, 4> vkPipelineStageFlags;

				for (auto& si : submitInfos) {
					auto& a = vkSubmitInfos.EmplaceBack(VK_STRUCTURE_TYPE_SUBMIT_INFO);
					auto& wucb = vkCommandBuffers.EmplaceBack();
					auto& wuss = signalSemaphores.EmplaceBack(); auto& wuws = waitSemaphores.EmplaceBack();
					auto& psfs = vkPipelineStageFlags.EmplaceBack();

					for (auto cbi : si.CommandLists) {
						auto& cb = wucb.EmplaceBack(static_cast<const VulkanCommandList*>(cbi)->GetVkCommandBuffer());
					}

					for (auto& cbi : si.SignalSemaphores) {
						auto& s = wuss.EmplaceBack(static_cast<const VulkanSemaphore*>(cbi.Semaphore)->GetVkSemaphore());
						static_cast<VulkanSemaphore*>(cbi.Semaphore)->Signal();
					}

					for (auto& cbi : si.WaitSemaphores) {
						auto& s = wuws.EmplaceBack(static_cast<const VulkanSemaphore*>(cbi.Semaphore)->GetVkSemaphore());
						psfs.EmplaceBack(ToVulkan(cbi.PipelineStage));
						static_cast<VulkanSemaphore*>(cbi.Semaphore)->Unsignal();
					}

					a.commandBufferCount = wucb.GetLength();
					a.pCommandBuffers = wucb.begin();
					a.waitSemaphoreCount = wuws.GetLength();
					a.pWaitSemaphores = wuws.begin();
					a.signalSemaphoreCount = wuss.GetLength();
					a.pSignalSemaphores = wuss.begin();
					a.pWaitDstStageMask = psfs.GetData();
				}

				submitResult = renderDevice->VkQueueSubmit(queue, vkSubmitInfos.GetLength(), vkSubmitInfos.GetData(), fence.GetVkFence());
			}

			fence.Signal();

			if(submitResult == VK_ERROR_DEVICE_LOST) {
				renderDevice->Log(u8"Error: Device lost", RenderDevice::MessageSeverity::ERROR);
			}

			return submitResult == VK_SUCCESS;
		}

		[[nodiscard]] VkQueue GetVkQueue() const { return queue; }
		[[nodiscard]] VulkanRenderDevice::QueueKey GetQueueKey() const { return queueKey; }

	private:
		VkQueue queue = nullptr;
		VulkanRenderDevice::QueueKey queueKey;
	};
}
