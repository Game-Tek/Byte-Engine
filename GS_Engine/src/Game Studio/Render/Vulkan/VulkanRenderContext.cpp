#include "Vulkan.h"

#include "VulkanRenderContext.h"
#include "VulkanRenderPass.h"
#include "VulkanFramebuffer.h"
#include "VulkanPipelines.h"

#include "..\Window.h"
#include "..\Platform\Windows\WindowsWindow.h"

//  VULKAN RENDER CONTEXT

VulkanRenderContext::VulkanRenderContext(VkDevice _Device, VkInstance _Instance, VkPhysicalDevice _PD, Window* _Window,VkQueue _PresentationQueue, uint32 _PresentationQueueIndex) : 
	Surface(_Device, _Instance, _PD, _Window),
	Swapchain(_Device, _PD, Surface.GetVkSurface(), Surface.GetVkSurfaceFormat(), Surface.GetVkColorSpaceKHR(), Surface.GetVkExtent2D()),
	PresentationQueue(_PresentationQueue),
	ImageAvailable(_Device),
	RenderFinished(_Device),
	CommandPool(_Device, _PresentationQueueIndex),
	CommandBuffer{ { _Device, CommandPool.GetVkCommandPool() }, { _Device, CommandPool.GetVkCommandPool() }, { _Device, CommandPool.GetVkCommandPool() } }
{
}

void VulkanRenderContext::Present()
{
	/* Initialize semaphores */
	VkPipelineStageFlags WaitStages[] = { VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT };
	VkSemaphore WaitSemaphores[] = { ImageAvailable.GetVkSemaphore() };
	VkSemaphore SignalSemaphores[] = { RenderFinished.GetVkSemaphore() };

	/* Submit signal semaphore to graphics queue */
	VkSubmitInfo submitInfo = { VK_STRUCTURE_TYPE_SUBMIT_INFO };
	{
		submitInfo.waitSemaphoreCount = 1;
		submitInfo.pWaitSemaphores = WaitSemaphores;
		submitInfo.pWaitDstStageMask = WaitStages;
		submitInfo.commandBufferCount = 0;
		submitInfo.pCommandBuffers = nullptr;
		submitInfo.signalSemaphoreCount = 1;
		submitInfo.pSignalSemaphores = SignalSemaphores;
	}
	
	GS_VK_CHECK(vkQueueSubmit(PresentationQueue, 1, &submitInfo, VK_NULL_HANDLE), "Failed to Submit!")

	/* Present result on screen */
	VkSwapchainKHR Swapchains[] = { Swapchain.GetVkSwapchain() };

	uint32 ImageIndex = Swapchain.AcquireNextImage(ImageAvailable.GetVkSemaphore());

	VkPresentInfoKHR PresentInfo = { VK_STRUCTURE_TYPE_PRESENT_INFO_KHR };
	{
		PresentInfo.waitSemaphoreCount = 1;
		PresentInfo.pWaitSemaphores = WaitSemaphores;
		PresentInfo.swapchainCount = 1;
		PresentInfo.pSwapchains = Swapchains;
		PresentInfo.pImageIndices = &ImageIndex;
		PresentInfo.pResults = nullptr;
	}

	GS_VK_CHECK(vkQueuePresentKHR(PresentationQueue, &PresentInfo), "Failed to present Vulkan graphics queue!")

	CurrentCommandBufferIndex = (CurrentCommandBufferIndex + 1) % Swapchain.GetImages().length();
}

void VulkanRenderContext::BeginRecording()
{
	VkCommandBufferBeginInfo BeginInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
	BeginInfo.flags = VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT;
	//Hint to primary buffer if this is secondary.
	BeginInfo.pInheritanceInfo = nullptr;

	GS_VK_CHECK(vkBeginCommandBuffer(CommandBuffer[CurrentCommandBufferIndex].GetVkCommandBuffer(), &BeginInfo), "Failed to begin Command Buffer!")
}

void VulkanRenderContext::EndRecording()
{
	GS_VK_CHECK(vkEndCommandBuffer(CommandBuffer[CurrentCommandBufferIndex].GetVkCommandBuffer()), "Failed to end Command Buffer!")
}

void VulkanRenderContext::BeginRenderPass(const RenderPassBeginInfo& _RPBI)
{
	VkClearValue ClearColor = { 0.0f, 0.0f, 0.0f, 0.0f };

	VkRenderPassBeginInfo RenderPassBeginInfo = { VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO };
	RenderPassBeginInfo.renderPass = SCAST(VulkanRenderPass*, _RPBI.RenderPass)->GetVk_RenderPass().GetVkRenderPass();
	RenderPassBeginInfo.pClearValues = &ClearColor;
	RenderPassBeginInfo.clearValueCount = 1;
	RenderPassBeginInfo.framebuffer = SCAST(VulkanFramebuffer*, _RPBI.Framebuffer)->GetVk_Framebuffer().GetVkFramebuffer();
	RenderPassBeginInfo.renderArea.extent = Extent2DToVkExtent2D(_RPBI.RenderArea);
	RenderPassBeginInfo.renderArea.offset = { 0, 0 };

	vkCmdBeginRenderPass(CommandBuffer[CurrentCommandBufferIndex].GetVkCommandBuffer(), &RenderPassBeginInfo, VK_SUBPASS_CONTENTS_INLINE);
}

void VulkanRenderContext::EndRenderPass(RenderPass* _RP)
{
	vkCmdEndRenderPass(CommandBuffer[CurrentCommandBufferIndex].GetVkCommandBuffer());
}

void VulkanRenderContext::BindGraphicsPipeline(GraphicsPipeline* _GP)
{
	vkCmdBindPipeline(CommandBuffer[CurrentCommandBufferIndex].GetVkCommandBuffer(), VK_PIPELINE_BIND_POINT_GRAPHICS, SCAST(VulkanGraphicsPipeline*, _GP)->GetVk_GraphicsPipeline().GetVkGraphicsPipeline());
}

void VulkanRenderContext::BindComputePipeline(ComputePipeline* _CP)
{
	vkCmdBindPipeline(CommandBuffer[CurrentCommandBufferIndex].GetVkCommandBuffer(), VK_PIPELINE_BIND_POINT_COMPUTE, SCAST(VulkanComputePipeline*, _CP)->GetVk_ComputePipeline().GetVkPipeline());
}

void VulkanRenderContext::DrawIndexed(const DrawInfo& _DI)
{
	vkCmdDrawIndexed(CommandBuffer[CurrentCommandBufferIndex].GetVkCommandBuffer(), _DI.IndexCount, _DI.InstanceCount, 0, 0, 0);
}


//  VULKAN SWAPCHAIN

Vk_Swapchain::Vk_Swapchain(VkDevice _Device, VkPhysicalDevice _PD, VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent) : VulkanObject(_Device)
{
	FindPresentMode(PresentMode, _PD, _Surface);

	VkSwapchainCreateInfoKHR SwapchainCreateInfo;
	CreateSwapchainCreateInfo(SwapchainCreateInfo, _Surface, _SurfaceFormat, _SurfaceColorSpace, _SurfaceExtent, PresentMode, VK_NULL_HANDLE);

	GS_VK_CHECK(vkCreateSwapchainKHR(m_Device, &SwapchainCreateInfo, ALLOCATOR, &Swapchain), "Failed to create Swapchain!")

	uint32_t ImageCount = 0;
	vkGetSwapchainImagesKHR(m_Device, Swapchain, &ImageCount, nullptr);
	Images.resize(ImageCount);
	vkGetSwapchainImagesKHR(m_Device, Swapchain, &ImageCount, Images.data());
}

Vk_Swapchain::~Vk_Swapchain()
{
	vkDestroySwapchainKHR(m_Device, Swapchain, ALLOCATOR);
}

void Vk_Swapchain::Recreate(VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent)
{
	VkSwapchainCreateInfoKHR SwapchainCreateInfo;
	CreateSwapchainCreateInfo(SwapchainCreateInfo, _Surface, _SurfaceFormat, _SurfaceColorSpace, _SurfaceExtent, PresentMode, Swapchain);

	GS_VK_CHECK(vkCreateSwapchainKHR(m_Device, &SwapchainCreateInfo, ALLOCATOR, &Swapchain), "Failed to create Swapchain!")

	uint32_t ImageCount = 0;
	vkGetSwapchainImagesKHR(m_Device, Swapchain, &ImageCount, nullptr);
	Images.resize(ImageCount);
	vkGetSwapchainImagesKHR(m_Device, Swapchain, &ImageCount, Images.data());
}

uint32 Vk_Swapchain::AcquireNextImage(VkSemaphore _ImageAvailable)
{
	uint32 ImageIndex = 0;
	vkAcquireNextImageKHR(m_Device, Swapchain, 0xffffffffffffffff, _ImageAvailable, VK_NULL_HANDLE, &ImageIndex);
	return ImageIndex;
}

void Vk_Swapchain::CreateSwapchainCreateInfo(VkSwapchainCreateInfoKHR & _SCIK, VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode, VkSwapchainKHR _OldSwapchain)
{
	_SCIK.sType = VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR;
	_SCIK.surface = _Surface;
	_SCIK.minImageCount = 3;
	_SCIK.imageFormat = _SurfaceFormat;
	_SCIK.imageColorSpace = _SurfaceColorSpace;
	_SCIK.imageExtent = _SurfaceExtent;
	//The imageArrayLayers specifies the amount of layers each image consists of. This is always 1 unless you are developing a stereoscopic 3D application.
	_SCIK.imageArrayLayers = 1;
	//Should be VK_IMAGE_USAGE_TRANSFER_DST_BIT
	_SCIK.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
	_SCIK.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
	_SCIK.queueFamilyIndexCount = 1; // Optional
	_SCIK.pQueueFamilyIndices = nullptr;
	_SCIK.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
	//The compositeAlpha field specifies if the alpha channel should be used for blending with other windows in the window system.
	//You'll almost always want to simply ignore the alpha channel, hence VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR.
	_SCIK.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
	_SCIK.presentMode = _PresentMode;
	_SCIK.clipped = VK_TRUE;
	_SCIK.oldSwapchain = _OldSwapchain;
}
uint8 Vk_Swapchain::ScorePresentMode(VkPresentModeKHR _PresentMode)
{
	switch (_PresentMode)
	{
	case VK_PRESENT_MODE_MAILBOX_KHR:	return 255;
	case VK_PRESENT_MODE_FIFO_KHR:		return 254;
	default:							return 0;
	}
}
void Vk_Swapchain::FindPresentMode(VkPresentModeKHR& _PM, VkPhysicalDevice _PD, VkSurfaceKHR _Surface)
{
	uint32_t PresentModesCount = 0;
	vkGetPhysicalDeviceSurfacePresentModesKHR(_PD, _Surface, &PresentModesCount, nullptr);
	FVector<VkPresentModeKHR> PresentModes(PresentModesCount);
	vkGetPhysicalDeviceSurfacePresentModesKHR(_PD, _Surface, &PresentModesCount, PresentModes.data());

	uint8 BestScore = 0;
	uint8 BestPresentModeIndex = 0;
	for (uint8 i = 0; i < PresentModesCount; i++)
	{
		if (ScorePresentMode(PresentModes[i]) > BestScore)
		{
			BestScore = ScorePresentMode(PresentModes[i]);

			BestPresentModeIndex = i;
		}
	}

	_PM = PresentModes[BestPresentModeIndex];
}


// VULKAN SURFACE

Vk_Surface::Vk_Surface(VkDevice _Device, VkInstance _Instance, VkPhysicalDevice _PD, Window* _Window) : VulkanObject(_Device), m_Instance(_Instance)
{
	VkWin32SurfaceCreateInfoKHR WCreateInfo = { VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR };
	WCreateInfo.hwnd = SCAST(WindowsWindow*, _Window)->GetWindowObject();
	WCreateInfo.hinstance = GetModuleHandle(nullptr);

	GS_VK_CHECK(vkCreateWin32SurfaceKHR(m_Instance, &WCreateInfo, ALLOCATOR, &Surface), "Failed to create Windows Surface!")

	Format = PickBestFormat(_PD, Surface);
}

Vk_Surface::~Vk_Surface()
{
	vkDestroySurfaceKHR(m_Instance, Surface, ALLOCATOR);
}

VkFormat Vk_Surface::PickBestFormat(VkPhysicalDevice _PD, VkSurfaceKHR _Surface)
{
	uint32_t FormatsCount = 0;
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, nullptr);
	FVector<VkSurfaceFormatKHR> SurfaceFormats(FormatsCount);
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, SurfaceFormats.data());

	uint8 i = 0;
	if (SurfaceFormats[i].colorSpace == VK_COLOR_SPACE_SRGB_NONLINEAR_KHR && SurfaceFormats[i].format == VK_FORMAT_B8G8R8A8_UNORM)
	{
		return SurfaceFormats[i].format;
	}
}
