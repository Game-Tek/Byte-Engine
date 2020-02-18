#include "Vulkan.h"

#include "VulkanRenderDevice.h"

#include "VulkanRenderContext.h"
#include "VulkanRenderPass.h"
#include "VulkanFramebuffer.h"

#include "RAPI/Window.h"
#include "Native/vkPhysicalDevice.h"

//  VULKAN RENDER CONTEXT

using namespace RAPI;

uint8 ScorePresentMode(VkPresentModeKHR _PresentMode)
{
	switch (_PresentMode)
	{
	case VK_PRESENT_MODE_MAILBOX_KHR: return 255;
	case VK_PRESENT_MODE_FIFO_KHR: return 254;
	default: return 0;
	}
}

VKSurfaceCreator VulkanRenderContext::CreateSurface(VKDevice* _Device, VKInstance* _Instance, Window* _Window)
{
	return VKSurfaceCreator(_Device, _Instance, _Window);
}

VKSwapchainCreator VulkanRenderContext::CreateSwapchain(VKDevice* _Device, VkSwapchainKHR _OldSwapchain) const
{
	VkSwapchainCreateInfoKHR SwapchainCreateInfo = {VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR};

	SwapchainCreateInfo.surface = Surface.GetHandle();
	SwapchainCreateInfo.minImageCount = 3;
	SwapchainCreateInfo.imageFormat = Format.format;
	SwapchainCreateInfo.imageColorSpace = Format.colorSpace;
	SwapchainCreateInfo.imageExtent = Extent2DToVkExtent2D(RenderExtent);
	//The imageArrayLayers specifies the amount of layers each image consists of. This is always 1 unless you are developing a stereoscopic 3D application.
	SwapchainCreateInfo.imageArrayLayers = 1;
	SwapchainCreateInfo.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
	SwapchainCreateInfo.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
	SwapchainCreateInfo.queueFamilyIndexCount = 0;
	SwapchainCreateInfo.pQueueFamilyIndices = nullptr;
	SwapchainCreateInfo.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
	SwapchainCreateInfo.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
	SwapchainCreateInfo.presentMode = PresentMode;
	SwapchainCreateInfo.clipped = VK_TRUE;
	SwapchainCreateInfo.oldSwapchain = _OldSwapchain;

	return VKSwapchainCreator(_Device, &SwapchainCreateInfo);
}

VKCommandPoolCreator VulkanRenderContext::CreateCommandPool(VKDevice* _Device)
{
	VkCommandPoolCreateInfo CommandPoolCreateInfo = {VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
	CommandPoolCreateInfo.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;

	return VKCommandPoolCreator(_Device, &CommandPoolCreateInfo);
}

SurfaceFormat VulkanRenderContext::FindFormat(const vkPhysicalDevice& _PD, VkSurfaceKHR _Surface)
{
	uint32_t FormatsCount = 0;
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, nullptr);
	DArray<VkSurfaceFormatKHR> SurfaceFormats(FormatsCount);
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, SurfaceFormats.getData());

	//NASTY, REMOVE
	VkBool32 Supports = 0;
	vkGetPhysicalDeviceSurfaceSupportKHR(_PD, PresentationQueue.GetQueueIndex(), _Surface, &Supports);
	//NASTY, REMOVE

	VkSurfaceCapabilitiesKHR SurfaceCapabilities = {};
	vkGetPhysicalDeviceSurfaceCapabilitiesKHR(_PD, _Surface, &SurfaceCapabilities);

	VkBool32 Supported = 0;
	vkGetPhysicalDeviceSurfaceSupportKHR(_PD, PresentationQueue.GetQueueIndex(), _Surface, &Supported);

	return {SurfaceFormats[0].format, SurfaceFormats[0].colorSpace};
}

VkPresentModeKHR VulkanRenderContext::FindPresentMode(const vkPhysicalDevice& _PD, const VKSurface& _Surface)
{
	uint32_t PresentModesCount = 0;
	vkGetPhysicalDeviceSurfacePresentModesKHR(_PD, _Surface.GetHandle(), &PresentModesCount, nullptr);
	DArray<VkPresentModeKHR> PresentModes(PresentModesCount);
	vkGetPhysicalDeviceSurfacePresentModesKHR(_PD, _Surface.GetHandle(), &PresentModesCount, PresentModes.getData());

	uint8 BestScore = 0;
	uint8 BestPresentModeIndex = 0;
	for (uint8 i = 0; i < PresentModes.getLength(); i++)
	{
		if (ScorePresentMode(PresentModes[i]) > BestScore)
		{
			BestScore = ScorePresentMode(PresentModes[i]);

			BestPresentModeIndex = i;
		}
	}

	return PresentModes[BestPresentModeIndex];
}

VulkanRenderContext::VulkanRenderContext(VulkanRenderDevice* device, VKInstance* _Instance, const vkPhysicalDevice& _PD,
                                         Window* _Window) :
	RenderExtent(_Window->GetWindowExtent()),
	Surface(CreateSurface(&device->GetVKDevice(), _Instance, _Window)),
	Format(FindFormat(_PD, Surface)),
	PresentMode(FindPresentMode(_PD, Surface)),
	Swapchain(CreateSwapchain(&device->GetVKDevice(), VK_NULL_HANDLE)),
	SwapchainImages(Swapchain.GetImages()),
	swapchainImages(SwapchainImages.getCapacity()),
	ImagesAvailable(SwapchainImages.getCapacity()),
	RendersFinished(SwapchainImages.getCapacity()),
	InFlightFences(SwapchainImages.getCapacity()),
	PresentationQueue(device->GetVKDevice().GetGraphicsQueue()),
	CommandPool(CreateCommandPool(&device->GetVKDevice())),
	CommandBuffers(SwapchainImages.getCapacity()),
	FrameBuffers(SwapchainImages.getCapacity())

{
	MAX_FRAMES_IN_FLIGHT = SCAST(uint8, SwapchainImages.getCapacity());

	VkSemaphoreCreateInfo SemaphoreCreateInfo = {VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};

	VkFenceCreateInfo FenceCreateInfo = {VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
	FenceCreateInfo.flags = VK_FENCE_CREATE_SIGNALED_BIT;

	for (uint8 i = 0; i < MAX_FRAMES_IN_FLIGHT; ++i)
	{
		ImagesAvailable.emplace_back(VKSemaphoreCreator(&device->GetVKDevice(), &SemaphoreCreateInfo));
		RendersFinished.emplace_back(VKSemaphoreCreator(&device->GetVKDevice(), &SemaphoreCreateInfo));
		InFlightFences.emplace_back(VKFenceCreator(&device->GetVKDevice(), &FenceCreateInfo));
		CommandBuffers.emplace_back(CommandPool.CreateCommandBuffer());

		ImageCreateInfo image_create_info;
		image_create_info.Extent = {RenderExtent.Width, RenderExtent.Height, 0};
		image_create_info.ImageFormat = VkFormatToFormat(Format.format);

		swapchainImages.emplace_back(VulkanSwapchainImage(device, image_create_info, SwapchainImages[i]));
	}
}

VulkanRenderContext::~VulkanRenderContext() = default;

void VulkanRenderContext::OnResize(const ResizeInfo& _RI)
{
	RenderExtent = _RI.NewWindowSize;
	Swapchain.Recreate(Surface, Format.format, Format.colorSpace, Extent2DToVkExtent2D(_RI.NewWindowSize), PresentMode);
}

void VulkanRenderContext::AcquireNextImage()
{
	const auto lImageIndex = Swapchain.AcquireNextImage(ImagesAvailable[CurrentImage]);
	//This signals the semaphore when the image becomes available
	ImageIndex = lImageIndex;
}

void VulkanRenderContext::Flush()
{
	InFlightFences[CurrentImage].Wait(); //Get current's frame fences and wait for it.
	InFlightFences[CurrentImage].Reset(); //Then reset it.

	VkPipelineStageFlags WaitStages[] = {VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT};

	VkSemaphore WaitSemaphores[] = {ImagesAvailable[CurrentImage]};
	//Set current's frame ImageAvaiable semaphore as the semaphore to wait for to start rendering to.
	VkSemaphore SignalSemaphores[] = {RendersFinished[CurrentImage]};
	//Set current's frame RenderFinished semaphore as the semaphore to signal once rendering has finished.
	VkCommandBuffer lCommandBuffers[] = {CommandBuffers[CurrentImage]};

	/* Submit signal semaphore to graphics queue */
	VkSubmitInfo SubmitInfo = {VK_STRUCTURE_TYPE_SUBMIT_INFO};
	{
		SubmitInfo.waitSemaphoreCount = 1;
		SubmitInfo.pWaitSemaphores = WaitSemaphores;
		SubmitInfo.commandBufferCount = 1;
		SubmitInfo.pCommandBuffers = lCommandBuffers;
		SubmitInfo.signalSemaphoreCount = 1;
		SubmitInfo.pSignalSemaphores = SignalSemaphores;

		SubmitInfo.pWaitDstStageMask = WaitStages;
	}

	PresentationQueue.Submit(&SubmitInfo, InFlightFences[CurrentImage]);
	//Signal fence when execution of this queue has finished.

	InFlightFences[CurrentImage].Wait();
	CommandBuffers[CurrentImage].Reset();
}

void VulkanRenderContext::Present()
{
	VkSemaphore WaitSemaphores[] = {RendersFinished[CurrentImage]};

	/* Present result on screen */
	const VkSwapchainKHR Swapchains[] = {Swapchain};

	uint32 lImageIndex = ImageIndex;

	VkPresentInfoKHR PresentInfo = {VK_STRUCTURE_TYPE_PRESENT_INFO_KHR};
	{
		PresentInfo.waitSemaphoreCount = 1;
		PresentInfo.pWaitSemaphores = WaitSemaphores;
		PresentInfo.swapchainCount = 1;
		PresentInfo.pSwapchains = Swapchains;
		PresentInfo.pImageIndices = &lImageIndex;
		PresentInfo.pResults = nullptr;
	}

	PresentationQueue.Present(&PresentInfo);

	CurrentImage = (CurrentImage + 1) % MAX_FRAMES_IN_FLIGHT;
}


FVector<Image*> VulkanRenderContext::GetSwapchainImages() const
{
	FVector<Image*> l_Images(MAX_FRAMES_IN_FLIGHT);

	for (uint8 i = 0; i < MAX_FRAMES_IN_FLIGHT; ++i)
	{
		l_Images.push_back(static_cast<Image*>(&swapchainImages[i]));
	}

	return l_Images;
}
