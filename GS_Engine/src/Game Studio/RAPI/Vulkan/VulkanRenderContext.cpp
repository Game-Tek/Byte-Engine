#include "Vulkan.h"

#include "VulkanRenderDevice.h"

#include "VulkanRenderContext.h"
#include "VulkanRenderPass.h"
#include "VulkanFramebuffer.h"

#include "RAPI/Window.h"
#include "Native/vkPhysicalDevice.h"
#include "RAPI/Platform/Windows/WindowsWindow.h"

#ifdef GS_PLATFORM_WIN
#define VK_USE_PLATFORM_WIN32_KHR
#include <windows.h>
#include <vulkan/vulkan_win32.h>
#endif

//  VULKAN RENDER CONTEXT

using namespace RAPI;

VKSurfaceCreator VulkanRenderContext::CreateSurface(VKDevice* _Device, VKInstance* _Instance, Window* _Window)
{
	return VKSurfaceCreator(_Device, _Instance, _Window);
}

VKCommandPoolCreator VulkanRenderContext::CreateCommandPool(VKDevice* _Device)
{
	VkCommandPoolCreateInfo CommandPoolCreateInfo = {VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
	CommandPoolCreateInfo.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;

	return VKCommandPoolCreator(_Device, &CommandPoolCreateInfo);
}

VkSurfaceFormatKHR VulkanRenderContext::FindFormat(const VulkanRenderDevice* device, VkSurfaceKHR surface)
{
	VkPhysicalDevice pd = device->GetPhysicalDevice();
	
	uint32 formats_count = 0;
	vkGetPhysicalDeviceSurfaceFormatsKHR(pd, surface, &formats_count, nullptr);
	Array<VkSurfaceFormatKHR, 50, uint8> supported_surface_formats(formats_count);
	formats_count = supported_surface_formats.getCapacity();
	vkGetPhysicalDeviceSurfaceFormatsKHR(pd, surface, &formats_count, supported_surface_formats.getData());
	
	//NASTY, REMOVE
	VkBool32 supports = 0;
	vkGetPhysicalDeviceSurfaceSupportKHR(pd, 0, surface, &supports);
	//NASTY, REMOVE

	VkSurfaceCapabilitiesKHR SurfaceCapabilities{};
	vkGetPhysicalDeviceSurfaceCapabilitiesKHR(pd, surface, &SurfaceCapabilities);

	VkBool32 supported = 0;
	vkGetPhysicalDeviceSurfaceSupportKHR(pd, 0, surface, &supported);

	return supported_surface_formats[0];
}

VkPresentModeKHR VulkanRenderContext::FindPresentMode(const vkPhysicalDevice& _PD, VkSurfaceKHR _Surface)
{
	uint32 present_modes_count = 0;
	vkGetPhysicalDeviceSurfacePresentModesKHR(_PD, _Surface, &present_modes_count, nullptr);
	Array<VkPresentModeKHR, 10, uint8> supported_present_modes(present_modes_count);
	present_modes_count = supported_present_modes.getCapacity();
	vkGetPhysicalDeviceSurfacePresentModesKHR(_PD, _Surface, &present_modes_count, supported_present_modes.getData());

	uint8 best_score = 0;
	VkPresentModeKHR best_present_mode{};
	for (auto& e : supported_present_modes)
	{
		uint8 score = 0;
		switch (e)
		{
		case VK_PRESENT_MODE_MAILBOX_KHR: score = 255; break;
		case VK_PRESENT_MODE_FIFO_KHR: score = 254; break;
		case VK_PRESENT_MODE_IMMEDIATE_KHR: score = 253; break;
		default: score = 0;
		}
		
		if (score > best_score)
		{
			best_score = score;
			best_present_mode = e;
		}
	}

	return best_present_mode;
}

VulkanRenderContext::VulkanRenderContext(VulkanRenderDevice* device, const RenderContextCreateInfo& renderContextCreateInfo)
{
	GS_ASSERT(renderContextCreateInfo.DesiredFramesInFlight > vulkanSwapchainImages.getCapacity(), "Requested swapchain image count is more than what the engine can handle, please request less.")
	
	extent = renderContextCreateInfo.Window->GetWindowExtent();
	
	VkWin32SurfaceCreateInfoKHR vk_win32_surface_create_info_khr = { VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR };
	vk_win32_surface_create_info_khr.hwnd = static_cast<WindowsWindow*>(renderContextCreateInfo.Window)->GetWindowObject();
	vk_win32_surface_create_info_khr.hinstance = static_cast<WindowsWindow*>(renderContextCreateInfo.Window)->GetHInstance();
	GS_VK_CHECK(vkCreateWin32SurfaceKHR(device->GetVkInstance(), &vk_win32_surface_create_info_khr, ALLOCATOR, &surface), "Failed to create Win32 Surface!");

	surfaceFormat = FindFormat(device, surface);
	
	presentMode = FindPresentMode(device->GetPhysicalDevice(), surface);
	
	VkSwapchainCreateInfoKHR vk_swapchain_create_info_khr = { VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR };
	vk_swapchain_create_info_khr.surface = surface;
	vk_swapchain_create_info_khr.minImageCount = renderContextCreateInfo.DesiredFramesInFlight;
	vk_swapchain_create_info_khr.imageFormat = surfaceFormat.format;
	vk_swapchain_create_info_khr.imageColorSpace = surfaceFormat.colorSpace;
	vk_swapchain_create_info_khr.imageExtent = Extent2DToVkExtent2D(extent);
	//The imageArrayLayers specifies the amount of layers each image consists of. This is always 1 unless you are developing a stereoscopic 3D application.
	vk_swapchain_create_info_khr.imageArrayLayers = 1;
	vk_swapchain_create_info_khr.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
	vk_swapchain_create_info_khr.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
	vk_swapchain_create_info_khr.queueFamilyIndexCount = 0;
	vk_swapchain_create_info_khr.pQueueFamilyIndices = nullptr;
	vk_swapchain_create_info_khr.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
	vk_swapchain_create_info_khr.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
	vk_swapchain_create_info_khr.presentMode = presentMode;
	vk_swapchain_create_info_khr.clipped = VK_TRUE;
	vk_swapchain_create_info_khr.oldSwapchain = nullptr;

	vkCreateSwapchainKHR(device->GetVKDevice(), &vk_swapchain_create_info_khr, ALLOCATOR, &swapchain);

	uint32 swapchain_image_count = 0;
	vkGetSwapchainImagesKHR(device->GetVKDevice(), swapchain, &swapchain_image_count, nullptr);
	GS_ASSERT(swapchain_image_count > vulkanSwapchainImages.getCapacity(), "Created swapchain images are more than what the engine can handle, please create less.")
	vulkanSwapchainImages.resize(swapchain_image_count);
	vkGetSwapchainImagesKHR(device->GetVKDevice(), swapchain, &swapchain_image_count, vulkanSwapchainImages.getData());
	
	maxFramesInFlight = static_cast<uint8>(vulkanSwapchainImages.getCapacity());
	
	VkSemaphoreCreateInfo SemaphoreCreateInfo = {VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};

	VkFenceCreateInfo FenceCreateInfo = {VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
	FenceCreateInfo.flags = VK_FENCE_CREATE_SIGNALED_BIT;

	for (uint8 i = 0; i < maxFramesInFlight; ++i)
	{
		ImagesAvailable.emplace_back(VKSemaphoreCreator(&device->GetVKDevice(), &SemaphoreCreateInfo));
		RendersFinished.emplace_back(VKSemaphoreCreator(&device->GetVKDevice(), &SemaphoreCreateInfo));
		InFlightFences.emplace_back(VKFenceCreator(&device->GetVKDevice(), &FenceCreateInfo));

		RenderTarget::RenderTargetCreateInfo image_create_info;
		image_create_info.Extent = { extent.Width, extent.Height, 0};
		image_create_info.Format = VkFormatToImageFormat(surfaceFormat.format);
		swapchainImages.emplace_back(device, image_create_info, vulkanSwapchainImages[i]);
	}
}

VulkanRenderContext::~VulkanRenderContext() = default;

void VulkanRenderContext::OnResize(const ResizeInfo& _RI)
{
	extent = _RI.NewWindowSize;

	VkSwapchainCreateInfoKHR vk_swapchain_create_info_khr = { VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR };
	vk_swapchain_create_info_khr.surface = surface;
	vk_swapchain_create_info_khr.minImageCount = maxFramesInFlight;
	vk_swapchain_create_info_khr.imageFormat = surfaceFormat.format;
	vk_swapchain_create_info_khr.imageColorSpace = surfaceFormat.colorSpace;
	vk_swapchain_create_info_khr.imageExtent = Extent2DToVkExtent2D(extent);
	//The imageArrayLayers specifies the amount of layers each image consists of. This is always 1 unless you are developing a stereoscopic 3D application.
	vk_swapchain_create_info_khr.imageArrayLayers = 1;
	vk_swapchain_create_info_khr.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
	vk_swapchain_create_info_khr.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
	vk_swapchain_create_info_khr.queueFamilyIndexCount = 0;
	vk_swapchain_create_info_khr.pQueueFamilyIndices = nullptr;
	vk_swapchain_create_info_khr.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
	vk_swapchain_create_info_khr.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
	vk_swapchain_create_info_khr.presentMode = presentMode;
	vk_swapchain_create_info_khr.clipped = VK_TRUE;
	vk_swapchain_create_info_khr.oldSwapchain = swapchain;

	vkCreateSwapchainKHR(static_cast<VulkanRenderDevice*>(_RI.RenderDevice)->GetVKDevice(), &vk_swapchain_create_info_khr, ALLOCATOR, &swapchain);
}

void VulkanRenderContext::AcquireNextImage(const AcquireNextImageInfo& acquireNextImageInfo)
{
	uint32 image_index = 0;

	vkAcquireNextImageKHR(static_cast<VulkanRenderDevice*>(acquireNextImageInfo.RenderDevice)->GetVKDevice().GetVkDevice, swapchain, ~0ULL, imagesAvailable[currentImage], nullptr, &image_index);

	//This signals the semaphore when the image becomes available
	imageIndex = image_index;
}

void RAPI::VulkanRenderContext::Flush(const FlushInfo& flushInfo)
{
	InFlightFences[currentImage].Wait(); //Get current's frame fences and wait for it.
	InFlightFences[currentImage].Reset(); //Then reset it.

	VkPipelineStageFlags WaitStages[] = { VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT };

	VkSemaphore WaitSemaphores[] = { ImagesAvailable[currentImage] };
	//Set current's frame ImageAvaiable semaphore as the semaphore to wait for to start rendering to.
	VkSemaphore SignalSemaphores[] = { RendersFinished[currentImage] };
	//Set current's frame RenderFinished semaphore as the semaphore to signal once rendering has finished.
	VkCommandBuffer lCommandBuffers[] = { CommandBuffers[currentImage] };

	/* Submit signal semaphore to graphics queue */
	VkSubmitInfo SubmitInfo = { VK_STRUCTURE_TYPE_SUBMIT_INFO };
	{
		SubmitInfo.waitSemaphoreCount = 1;
		SubmitInfo.pWaitSemaphores = WaitSemaphores;
		SubmitInfo.commandBufferCount = 1;
		SubmitInfo.pCommandBuffers = lCommandBuffers;
		SubmitInfo.signalSemaphoreCount = 1;
		SubmitInfo.pSignalSemaphores = SignalSemaphores;

		SubmitInfo.pWaitDstStageMask = WaitStages;
	}

	PresentationQueue.Submit(&SubmitInfo, InFlightFences[currentImage]);
	//Signal fence when execution of this queue has finished.

	InFlightFences[currentImage].Wait();
	CommandBuffers[currentImage].Reset();
}

void RAPI::VulkanRenderContext::Present(const PresentInfo& presentInfo)
{
	VkSemaphore WaitSemaphores[] = { RendersFinished[currentImage] };

	/* Present result on screen */
	const VkSwapchainKHR Swapchains[] = { Swapchain };

	uint32 lImageIndex = ImageIndex;

	VkPresentInfoKHR PresentInfo = { VK_STRUCTURE_TYPE_PRESENT_INFO_KHR };
	{
		PresentInfo.waitSemaphoreCount = 1;
		PresentInfo.pWaitSemaphores = WaitSemaphores;
		PresentInfo.swapchainCount = 1;
		PresentInfo.pSwapchains = Swapchains;
		PresentInfo.pImageIndices = &lImageIndex;
		PresentInfo.pResults = nullptr;
	}

	PresentationQueue.Present(&PresentInfo);

	currentImage = (currentImage + 1) % maxFramesInFlight;
}

FVector<RenderTarget*> VulkanRenderContext::GetSwapchainImages() const
{
	FVector<RenderTarget*> l_Images(maxFramesInFlight);

	for (uint8 i = 0; i < maxFramesInFlight; ++i)
	{
		l_Images.push_back(static_cast<RenderTarget*>(&swapchainImages[i]));
	}

	return l_Images;
}
