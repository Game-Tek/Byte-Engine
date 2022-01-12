#pragma once

#include "GAL/Synchronization.h"
#include "Vulkan.h"
#include "GTSL/Range.hpp"
#include "VulkanRenderDevice.h"

namespace GAL
{
	class VulkanFence final : public Fence
	{
	public:
		VulkanFence() = default;
		
		void Initialize(const VulkanRenderDevice* renderDevice, bool isSignaled = false) {
			VkFenceCreateInfo vk_fence_create_info{ VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
			vk_fence_create_info.flags = isSignaled;

			renderDevice->VkCreateFence(renderDevice->GetVkDevice(), &vk_fence_create_info, renderDevice->GetVkAllocationCallbacks(), &fence);
			counter = isSignaled ? 1 : 0;
		}

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyFence(renderDevice->GetVkDevice(), fence, renderDevice->GetVkAllocationCallbacks());
			debugClear(fence);
		}
		
		[[nodiscard]] VkFence GetVkFence() const { return fence; }

		void Wait(const VulkanRenderDevice* renderDevice) {
			if (State()) {
				auto result = renderDevice->VkWaitForFences(renderDevice->GetVkDevice(), 1u, &fence, true, 0xFFFFFFFFFFFFFFFF);
				if (result == VK_ERROR_DEVICE_LOST) {
					renderDevice->Log(u8"Error: device lost", RenderDevice::MessageSeverity::ERROR);
				}
			}
		}

		void Reset(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkResetFences(renderDevice->GetVkDevice(), 1u, &fence);
			Release();
		}
		
		[[nodiscard]] bool GetStatus(const VulkanRenderDevice* renderDevice) const {
			return counter;
		}

		void Signal() {
			++counter;
		}

		void Release() {
			--counter;
		}

		bool State() const { return counter; }
	private:
		VkFence fence = nullptr;
		GTSL::uint64 counter = 0;
	};

	class VulkanSemaphore final : public Semaphore
	{
	public:
		VulkanSemaphore() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, const GTSL::uint64 initialValue = 0xFFFFFFFFFFFFFFFF) {

			VkSemaphoreCreateInfo vkSemaphoreCreateInfo{ VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO };
			VkSemaphoreTypeCreateInfo vkSemaphoreTypeCreateInfo{ VK_STRUCTURE_TYPE_SEMAPHORE_TYPE_CREATE_INFO };
			vkSemaphoreTypeCreateInfo.semaphoreType = initialValue == 0xFFFFFFFFFFFFFFFF ? VK_SEMAPHORE_TYPE_BINARY : VK_SEMAPHORE_TYPE_TIMELINE;
			vkSemaphoreTypeCreateInfo.initialValue = initialValue == 0xFFFFFFFFFFFFFFFF ? 0 : initialValue;
			vkSemaphoreCreateInfo.pNext = &vkSemaphoreTypeCreateInfo;

			renderDevice->VkCreateSemaphore(renderDevice->GetVkDevice(), &vkSemaphoreCreateInfo, renderDevice->GetVkAllocationCallbacks(), &semaphore);
		}

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroySemaphore(renderDevice->GetVkDevice(), semaphore, renderDevice->GetVkAllocationCallbacks());
			debugClear(semaphore);
		}

		static void Wait(const VulkanRenderDevice* render_device, const GTSL::Range<const VulkanSemaphore*> semaphores, const GTSL::Range<const uint64*> values) {
			GTSL::StaticVector<VkSemaphore, 8> vk_semaphores;

			for(auto& e : semaphores) {
				vk_semaphores.EmplaceBack(e.GetVkSemaphore());
			}

			VkSemaphoreWaitInfo vk_semaphore_wait_info{ VK_STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO, nullptr, 0, vk_semaphores.GetLength(), vk_semaphores.GetData(), values.begin() };
			render_device->vkWaitSemaphores(render_device->GetVkDevice(), &vk_semaphore_wait_info, ~0ULL);
		}

		[[nodiscard]] VkSemaphore GetVkSemaphore() const { return semaphore; }

		void Signal() { ++counter; }
		void Unsignal() { --counter; }
		bool IsSignaled() const { return counter; }
	private:
		VkSemaphore semaphore{ nullptr };
		GTSL::uint64 counter = 0;
	};

	class VulkanEvent final : public Fence
	{
	public:
		VulkanEvent() = default;
		
		void Initialize(const VulkanRenderDevice* renderDevice) {
			VkEventCreateInfo vkEventCreateInfo{ VK_STRUCTURE_TYPE_EVENT_CREATE_INFO };
			renderDevice->VkCreateEvent(renderDevice->GetVkDevice(), &vkEventCreateInfo, renderDevice->GetVkAllocationCallbacks(), &event);
		}
		
		void Initialize(const VulkanRenderDevice* renderDevice, const GTSL::Range<const char8_t*> name) {
			VkEventCreateInfo vkEventCreateInfo{ VK_STRUCTURE_TYPE_EVENT_CREATE_INFO };
			renderDevice->VkCreateEvent(renderDevice->GetVkDevice(), &vkEventCreateInfo, renderDevice->GetVkAllocationCallbacks(), &event);

			setName(renderDevice, event, VK_OBJECT_TYPE_EVENT, name);
		}

		void Set(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkSetEvent(renderDevice->GetVkDevice(), event);
		}
		
		void Reset(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkResetEvent(renderDevice->GetVkDevice(), event);
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyEvent(renderDevice->GetVkDevice(), event, renderDevice->GetVkAllocationCallbacks());
			debugClear(event);
		}
		
		VkEvent GetVkEvent() const { return event; }

		GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(event); }
	private:
		VkEvent event = nullptr;
	};
}