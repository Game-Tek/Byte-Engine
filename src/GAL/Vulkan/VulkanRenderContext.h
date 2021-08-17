#pragma once

#include "GAL/RenderContext.h"

#include "VulkanTexture.h"
#include <GTSL/Pair.h>

#include "VulkanQueue.h"
#include "VulkanSynchronization.h"
#include "GTSL/Application.h"
#include "GTSL/Window.h"

namespace GAL
{
	class VulkanQueue;
	class VulkanSemaphore;

	class VulkanSurface final : public Surface
	{
	public:
		VulkanSurface() = default;
		
		bool Initialize(const VulkanRenderDevice* renderDevice, const GTSL::Application& application, const GTSL::Window& window) {
			VkWin32SurfaceCreateInfoKHR vkWin32SurfaceCreateInfoKhr{ VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR };
			vkWin32SurfaceCreateInfoKhr.hwnd = window.GetHWND();
			vkWin32SurfaceCreateInfoKhr.hinstance = application.GetHINSTANCE();
			return renderDevice->VkCreateWin32Surface(renderDevice->GetVkInstance(), &vkWin32SurfaceCreateInfoKhr, renderDevice->GetVkAllocationCallbacks(), &surface) == VK_SUCCESS;
			//setName(renderDevice, surface, VK_OBJECT_TYPE_SURFACE_KHR, createInfo.Name);
		}

		void Destroy(class VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroySurface(renderDevice->GetVkInstance(), surface, renderDevice->GetVkAllocationCallbacks());
			debugClear(surface);
		}

		GTSL::StaticVector<GTSL::Pair<ColorSpace, FormatDescriptor>, 16> GetSupportedFormatsAndColorSpaces(const VulkanRenderDevice* renderDevice) const {
			GTSL::uint32 surfaceFormatsCount = 16;
			VkSurfaceFormatKHR vkSurfaceFormatKhrs[16];
			renderDevice->VkGetPhysicalDeviceSurfaceFormats(renderDevice->GetVkPhysicalDevice(), surface, &surfaceFormatsCount, vkSurfaceFormatKhrs);

			GTSL::StaticVector<GTSL::Pair<ColorSpace, FormatDescriptor>, 16> result;

			for (GTSL::uint8 i = 0; i < static_cast<GTSL::uint8>(surfaceFormatsCount); ++i) {
				if(GAL::IsSupported(vkSurfaceFormatKhrs[i].format))
					result.EmplaceBack(GTSL::Pair(ToGAL(vkSurfaceFormatKhrs[i].colorSpace), ToGAL(vkSurfaceFormatKhrs[i].format)));
			}

			return result;
		}

		GTSL::StaticVector<PresentModes, 8> GetSupportedPresentModes(const VulkanRenderDevice* renderDevice) const {
			GTSL::uint32 presentModesCount = 8;
			VkPresentModeKHR vkPresentModes[8];
			renderDevice->VkGetPhysicalDeviceSurfacePresentModes(renderDevice->GetVkPhysicalDevice(), surface, &presentModesCount, vkPresentModes);

			GTSL::StaticVector<PresentModes, 8> result;

			for (GTSL::uint8 i = 0; i < static_cast<GTSL::uint8>(presentModesCount); ++i) {
				result.EmplaceBack(ToGAL(vkPresentModes[i]));
			}

			return result;
		}

		struct SurfaceCapabilities
		{
			uint32_t MinImageCount, MaxImageCount;
			GTSL::Extent2D CurrentExtent, MinImageExtent, MaxImageExtent;
			VkImageUsageFlags SupportedUsageFlags;
		};
		bool IsSupported(class VulkanRenderDevice* renderDevice, SurfaceCapabilities* surfaceCapabilities) {
			VkBool32 supported = 0;
			renderDevice->VkGetPhysicalDeviceSurfaceSupport(renderDevice->GetVkPhysicalDevice(), 0, surface, &supported);

			VkSurfaceCapabilitiesKHR vkSurfaceCapabilitiesKhr;
			renderDevice->VkGetPhysicalDeviceSurfaceCapabilities(renderDevice->GetVkPhysicalDevice(), surface, &vkSurfaceCapabilitiesKhr);

			auto vkExtentToExtent = [](const VkExtent2D vkExtent) { return GTSL::Extent2D(vkExtent.width, vkExtent.height); };

			surfaceCapabilities->CurrentExtent = vkExtentToExtent(vkSurfaceCapabilitiesKhr.currentExtent);
			surfaceCapabilities->MinImageExtent = vkExtentToExtent(vkSurfaceCapabilitiesKhr.minImageExtent);
			surfaceCapabilities->MaxImageExtent = vkExtentToExtent(vkSurfaceCapabilitiesKhr.maxImageExtent);
			surfaceCapabilities->MinImageCount = vkSurfaceCapabilitiesKhr.minImageCount;
			surfaceCapabilities->MaxImageCount = vkSurfaceCapabilitiesKhr.maxImageCount;
			surfaceCapabilities->SupportedUsageFlags = vkSurfaceCapabilitiesKhr.supportedUsageFlags;

			return supported;
		}

		[[nodiscard]] VkSurfaceKHR GetVkSurface() const { return surface; }
		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(surface); }
	
	private:
		VkSurfaceKHR surface = nullptr;
	};

	class VulkanRenderContext final : public RenderContext
	{
	public:
		VulkanRenderContext() = default;

		~VulkanRenderContext() = default;

		bool InitializeOrRecreate(const VulkanRenderDevice* renderDevice, [[maybe_unused]] const VulkanQueue queue, const VulkanSurface* surface,
		                          GTSL::Extent2D extent, FormatDescriptor format, ColorSpace colorSpace,
		                          TextureUse textureUse, PresentModes presentMode, GTSL::uint8 desiredFramesInFlight) {
			VkSwapchainCreateInfoKHR vkSwapchainCreateInfoKhr{ VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR };
			vkSwapchainCreateInfoKhr.surface = static_cast<VkSurfaceKHR>(surface->GetVkSurface());
			vkSwapchainCreateInfoKhr.minImageCount = desiredFramesInFlight;
			vkSwapchainCreateInfoKhr.imageFormat = ToVulkan(MakeFormatFromFormatDescriptor(format));
			vkSwapchainCreateInfoKhr.imageColorSpace = ToVulkan(colorSpace);
			vkSwapchainCreateInfoKhr.imageExtent = ToVulkan(extent);
			//The imageArrayLayers specifies the amount of layers each image consists of. This is always 1 unless you are developing a stereoscopic 3D application.
			vkSwapchainCreateInfoKhr.imageArrayLayers = 1;
			vkSwapchainCreateInfoKhr.imageUsage = ToVulkan(textureUse, format);
			vkSwapchainCreateInfoKhr.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
			vkSwapchainCreateInfoKhr.queueFamilyIndexCount = 0;
			vkSwapchainCreateInfoKhr.pQueueFamilyIndices = nullptr;
			vkSwapchainCreateInfoKhr.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
			vkSwapchainCreateInfoKhr.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
			vkSwapchainCreateInfoKhr.presentMode = ToVulkan(presentMode);
			vkSwapchainCreateInfoKhr.clipped = VK_TRUE;
			vkSwapchainCreateInfoKhr.oldSwapchain = swapchain;

			auto res = renderDevice->VkCreateSwapchain(renderDevice->GetVkDevice(), &vkSwapchainCreateInfoKhr, renderDevice->GetVkAllocationCallbacks(), &swapchain);
			//setName(createInfo.RenderDevice, swapchain, VK_OBJECT_TYPE_SWAPCHAIN_KHR, createInfo.Name);

			renderDevice->VkDestroySwapchain(renderDevice->GetVkDevice(), vkSwapchainCreateInfoKhr.oldSwapchain, renderDevice->GetVkAllocationCallbacks());

			return res == VK_SUCCESS;
		}

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroySwapchain(renderDevice->GetVkDevice(), swapchain, renderDevice->GetVkAllocationCallbacks());
			debugClear(swapchain);
		}

		enum class AcquireState { OK, SUBOPTIMAL, BAD };
		
		/**
		 * \brief  Acquires the next image in the swapchain queue to present to.
		 * \param acquireNextImageInfo Information to perform image acquisition.
		 * \return Returns true if the contexts needs to be recreated.
		 */
		[[nodiscard]] GTSL::Result<GTSL::uint8, AcquireState> AcquireNextImage(const VulkanRenderDevice* renderDevice, VulkanSemaphore& semaphore, VulkanFence& fence) {
			GTSL::uint32 image_index = 0;

			auto result = renderDevice->VkAcquireNextImage(renderDevice->GetVkDevice(), swapchain, ~0ULL, semaphore.GetVkSemaphore(), fence.GetVkFence(), &image_index);

			auto state = result == VK_SUCCESS ? AcquireState::OK : result == VK_SUBOPTIMAL_KHR ? AcquireState::SUBOPTIMAL : AcquireState::BAD;

			if(state != AcquireState::BAD) {
				fence.Signal();
				semaphore.Signal();
			}
			
			return GTSL::Result(static_cast<GTSL::uint8>(image_index), state);
		}
		
		[[nodiscard]] GTSL::Result<GTSL::uint8, AcquireState> AcquireNextImage(const VulkanRenderDevice* renderDevice, VulkanSemaphore& semaphore) {
			GTSL::uint32 image_index = 0;

			auto result = renderDevice->VkAcquireNextImage(renderDevice->GetVkDevice(), swapchain, ~0ULL, semaphore.GetVkSemaphore(), nullptr, &image_index);

			auto state = result == VK_SUCCESS ? AcquireState::OK : result == VK_SUBOPTIMAL_KHR ? AcquireState::SUBOPTIMAL : AcquireState::BAD;

			if (state != AcquireState::BAD) {
				if (!semaphore.IsSignaled()) {
					semaphore.Signal();
				}
			}
			
			return GTSL::Result(static_cast<GTSL::uint8>(image_index), state);
		}
		
		bool Present(const VulkanRenderDevice* renderDevice, GTSL::Range<VulkanSemaphore**> waitSemaphores, GTSL::uint32 index, VulkanQueue queue) {
			VkPresentInfoKHR vkPresentInfoKhr{ VK_STRUCTURE_TYPE_PRESENT_INFO_KHR };

			GTSL::StaticVector<VkSemaphore, 16> semaphores;

			for (auto& s : waitSemaphores) {
				if (s->IsSignaled()) {
					s->Reset();
					semaphores.EmplaceBack(s->GetVkSemaphore());
				}
			}
			
			vkPresentInfoKhr.waitSemaphoreCount = semaphores.GetLength();
			vkPresentInfoKhr.pWaitSemaphores = semaphores.begin();
			vkPresentInfoKhr.swapchainCount = 1;
			vkPresentInfoKhr.pSwapchains = &swapchain;
			vkPresentInfoKhr.pImageIndices = &index;
			vkPresentInfoKhr.pResults = nullptr;

			return renderDevice->VkQueuePresent(queue.GetVkQueue(), &vkPresentInfoKhr) == VK_SUCCESS;
		}

		[[nodiscard]] GTSL::StaticVector<VulkanTexture, 8> GetTextures(const VulkanRenderDevice* renderDevice) const {
			GTSL::uint32 swapchainImageCount = 8;
			VkImage vkImages[8];
			renderDevice->VkGetSwapchainImages(renderDevice->GetVkDevice(), swapchain, &swapchainImageCount, vkImages);

			GTSL::StaticVector<VulkanTexture, 8> vulkanTextures;
			
			for(GTSL::uint32 i = 0; i < swapchainImageCount; ++i) {
				vulkanTextures.EmplaceBack(vkImages[i]);
			}
			
			return vulkanTextures;
		}

		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(swapchain); }
	
	private:
		VkSwapchainKHR swapchain = nullptr;
	};
}
