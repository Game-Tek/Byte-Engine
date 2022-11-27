#pragma once

#include "GAL/Synchronization.h"
#include "Vulkan.h"
#include "GTSL/Range.hpp"
#include "VulkanRenderDevice.h"

namespace GAL
{
	class VulkanSynchronizer final : public Synchronizer {
	public:
		VulkanSynchronizer() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, const GTSL::StringView name, Type syncType, bool isSignaled = false, uint64 initialValue = ~0ULL) {
			SyncType = syncType;

			switch (SyncType) {
			case Type::FENCE: {
				VkFenceCreateInfo vk_fence_create_info{ VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
				vk_fence_create_info.flags = isSignaled;

				renderDevice->VkCreateFence(renderDevice->GetVkDevice(), &vk_fence_create_info, renderDevice->GetVkAllocationCallbacks(), &fence);
				counter = isSignaled ? 1 : 0;
				break;
			}
			case Type::SEMAPHORE: {
				VkSemaphoreCreateInfo vkSemaphoreCreateInfo{ VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO };
				VkSemaphoreTypeCreateInfo vkSemaphoreTypeCreateInfo{ VK_STRUCTURE_TYPE_SEMAPHORE_TYPE_CREATE_INFO };
				vkSemaphoreTypeCreateInfo.semaphoreType = initialValue == 0xFFFFFFFFFFFFFFFF ? VK_SEMAPHORE_TYPE_BINARY : VK_SEMAPHORE_TYPE_TIMELINE;
				vkSemaphoreTypeCreateInfo.initialValue = initialValue == 0xFFFFFFFFFFFFFFFF ? 0 : initialValue;
				vkSemaphoreCreateInfo.pNext = &vkSemaphoreTypeCreateInfo;

				renderDevice->VkCreateSemaphore(renderDevice->GetVkDevice(), &vkSemaphoreCreateInfo, renderDevice->GetVkAllocationCallbacks(), &semaphore);

				setName(renderDevice, semaphore, VK_OBJECT_TYPE_SEMAPHORE, name);
				break;
			}
			case Type::EVENT: {
				VkEventCreateInfo vkEventCreateInfo{ VK_STRUCTURE_TYPE_EVENT_CREATE_INFO };
				renderDevice->VkCreateEvent(renderDevice->GetVkDevice(), &vkEventCreateInfo, renderDevice->GetVkAllocationCallbacks(), &event);
				break;
			}
			}
		}
		
		[[nodiscard]] VkFence GetVkFence() const { return fence; }
		[[nodiscard]] VkSemaphore GetVkSemaphore() const { return semaphore; }
		[[nodiscard]] VkEvent GetVkEvent() const { return event; }

		void Wait(const VulkanRenderDevice* renderDevice) {
			if (State()) {
				auto result = renderDevice->VkWaitForFences(renderDevice->GetVkDevice(), 1u, &fence, true, 0xFFFFFFFFFFFFFFFF);
				if (result == VK_ERROR_DEVICE_LOST) {
					renderDevice->Log(u8"Error: device lost", RenderDevice::MessageSeverity::ERROR);
				}
			}
		}

		void Reset(const VulkanRenderDevice* renderDevice) {
			switch (SyncType) {
			case Type::FENCE: renderDevice->VkResetFences(renderDevice->GetVkDevice(), 1u, &fence); break;
			case Type::SEMAPHORE: break;
			case Type::EVENT: renderDevice->VkResetEvent(renderDevice->GetVkDevice(), event); break;
			}
			
			Release();
		}

		void Signal() {
			++counter;
		}

		void Release() {
			--counter;
		}

		void Set(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkSetEvent(renderDevice->GetVkDevice(), event);
		}

		void Destroy(const VulkanRenderDevice* renderDevice) {
			switch (SyncType) {
			case Type::FENCE: renderDevice->VkDestroyFence(renderDevice->GetVkDevice(), fence, renderDevice->GetVkAllocationCallbacks()); debugClear(fence); break;
			case Type::SEMAPHORE : renderDevice->VkDestroySemaphore(renderDevice->GetVkDevice(), semaphore, renderDevice->GetVkAllocationCallbacks()); debugClear(semaphore); break;
			case Type::EVENT: renderDevice->VkDestroyEvent(renderDevice->GetVkDevice(), event, renderDevice->GetVkAllocationCallbacks()); debugClear(event); break;
			}
		}

		static void Wait(const VulkanRenderDevice* render_device, const GTSL::Range<const VulkanSynchronizer*> semaphores, const GTSL::Range<const uint64_t*> values) {
			GTSL::StaticVector<VkSemaphore, 8> vk_semaphores;

			for (auto& e : semaphores) {
				vk_semaphores.EmplaceBack(e.GetVkSemaphore());
			}

			VkSemaphoreWaitInfo vk_semaphore_wait_info{ VK_STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO, nullptr, 0, vk_semaphores.GetLength(), vk_semaphores.GetData(), values.begin() };
			render_device->vkWaitSemaphores(render_device->GetVkDevice(), &vk_semaphore_wait_info, ~0ULL);
		}

		bool State() const { return counter; }
	private:
		VkFence fence = nullptr;
		VkSemaphore semaphore = nullptr;
		VkEvent event = nullptr;
		GTSL::uint64 counter = 0;

		Type SyncType;
	};
}
