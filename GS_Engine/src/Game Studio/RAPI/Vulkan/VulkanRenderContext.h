#pragma once

#include "Core.h"

#include "RAPI/RenderContext.h"

#include "Containers/FVector.hpp"
#include "Native/VKSurface.h"
#include "Native/VKSwapchain.h"
#include "Native/VKCommandPool.h"
#include "Native/VKCommandBuffer.h"
#include "Native/VKSemaphore.h"
#include "Native/VKFence.h"
#include "Native/vkQueue.h"
#include "VulkanPipelines.h"
#include "VulkanSwapchainImage.h"
#include "VulkanBindings.h"

class VKDevice;

enum VkPresentModeKHR;
enum VkFormat;
enum VkColorSpaceKHR;

namespace RAPI
{
	class Window;

	class VulkanRenderContext final : public RenderContext
	{
		VkSurfaceKHR surface = nullptr;
		VkSwapchainKHR swapchain = nullptr;

		VkSurfaceFormatKHR surfaceFormat{};
		VkPresentModeKHR presentMode{};

		Array<VkImage, 5, uint8> vulkanSwapchainImages;

		Array<VkSemaphore, 5, uint8> imagesAvailable;
		Array<VkSemaphore, 5, uint8> rendersFinished;
		Array<VkFence, 5, uint8> inFlightFences;
		
		mutable FVector<VulkanSwapchainImage> swapchainImages;
		FVector<VkSemaphore> ImagesAvailable;
		FVector<VkSemaphore> RendersFinished;
		FVector<VkFence> InFlightFences;

		uint8 imageIndex = 0;

		VkSurfaceFormatKHR FindFormat(const VulkanRenderDevice* device, VkSurfaceKHR surface);
		VkPresentModeKHR FindPresentMode(const vkPhysicalDevice& _PD, VkSurfaceKHR _Surface);
	public:
		VulkanRenderContext(VulkanRenderDevice* device, const RenderContextCreateInfo& renderContextCreateInfo);
		~VulkanRenderContext();

		void OnResize(const ResizeInfo& _RI) override;
		void AcquireNextImage(const AcquireNextImageInfo& acquireNextImageInfo) override;
		void Flush(const FlushInfo& flushInfo) override;
		void Present(const PresentInfo& presentInfo) override;

		[[nodiscard]] FVector<RenderTarget*> GetSwapchainImages() const override;
	};
}