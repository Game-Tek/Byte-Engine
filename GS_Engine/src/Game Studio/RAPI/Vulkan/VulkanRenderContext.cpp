#include "Vulkan.h"

#include "VulkanRenderer.h"

#include "VulkanRenderContext.h"
#include "VulkanRenderPass.h"
#include "VulkanFramebuffer.h"
#include "VulkanPipelines.h"

#include "RAPI/Window.h"
#include "RAPI/Platform/Windows/WindowsWindow.h"
#include "VulkanMesh.h"

//  VULKAN RENDER CONTEXT

VulkanRenderContext::VulkanRenderContext(const Vk_Device& _Device, VkInstance _Instance, VkPhysicalDevice _PD, Window* _Window) : 
	Surface(_Device, _Instance, _PD, _Window),
	Swapchain(_Device, _PD, Surface.GetVkSurface(), Surface.GetVkSurfaceFormat(), Surface.GetVkColorSpaceKHR(), Extent2DToVkExtent2D(_Window->GetWindowExtent())),
	ImageAvailable(_Device),
	RenderFinished(_Device),
	PresentationQueue(_Device.GetGraphicsQueue()),
	CommandPool(_Device, _Device.GetGraphicsQueue().GetQueueIndex()),
	MaxFramesInFlight(Swapchain.GetImages().length()),
	CommandBuffers(MaxFramesInFlight)
{
	for (uint8 i = 0; i < MaxFramesInFlight; i++)
	{
		Vk_CommandBuffer CB(_Device.GetVkDevice(), CommandPool.GetVkCommandPool());
		CommandBuffers.push_back(CB);
	}

}

void VulkanRenderContext::OnResize()
{
}

void VulkanRenderContext::Present()
{
	VkSemaphore WaitSemaphores[] = { ImageAvailable.GetVkSemaphore() };

	/* Present result on screen */
	const VkSwapchainKHR Swapchains[] = { Swapchain.GetVkSwapchain() };

	const uint32 ImageIndex = Swapchain.AcquireNextImage(ImageAvailable.GetVkSemaphore());
	CurrentImage = ImageIndex;

	VkPresentInfoKHR PresentInfo = { VK_STRUCTURE_TYPE_PRESENT_INFO_KHR };
	{
		PresentInfo.waitSemaphoreCount = 1;
		PresentInfo.pWaitSemaphores = WaitSemaphores;
		PresentInfo.swapchainCount = 1;
		PresentInfo.pSwapchains = Swapchains;
		PresentInfo.pImageIndices = &ImageIndex;
		PresentInfo.pResults = nullptr;
	}

	PresentationQueue.Present(&PresentInfo);
}

void VulkanRenderContext::Flush()
{
	VkPipelineStageFlags WaitStages[] = { VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT };
	VkSemaphore WaitSemaphores[] = { ImageAvailable.GetVkSemaphore() };
	VkSemaphore SignalSemaphores[] = { RenderFinished.GetVkSemaphore() };
	VkCommandBuffer lCommandBuffers[] = { CommandBuffers[CurrentImage] };

	//Each entry in the WaitStages array corresponds to the semaphore with the same index in WaitSemaphores.

	/* Submit signal semaphore to graphics queue */
	VkSubmitInfo SubmitInfo = { VK_STRUCTURE_TYPE_SUBMIT_INFO };
	SubmitInfo.waitSemaphoreCount = 1;
	SubmitInfo.pWaitSemaphores = WaitSemaphores;
	SubmitInfo.pWaitDstStageMask = WaitStages;
	SubmitInfo.commandBufferCount = 1;
	SubmitInfo.pCommandBuffers = lCommandBuffers;
	SubmitInfo.signalSemaphoreCount = 1;
	SubmitInfo.pSignalSemaphores = SignalSemaphores;

	PresentationQueue.Submit(&SubmitInfo, VK_NULL_HANDLE);
}

void VulkanRenderContext::BeginRecording()
{
	VkCommandBufferBeginInfo BeginInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
	BeginInfo.flags = VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT;
	//Hint to primary buffer if this is secondary.
	BeginInfo.pInheritanceInfo = nullptr;

	CommandBuffers[CurrentImage].Begin(&BeginInfo);
}

void VulkanRenderContext::EndRecording()
{
	CommandBuffers[CurrentImage].End();
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

	vkCmdBeginRenderPass(CommandBuffers[CurrentImage], &RenderPassBeginInfo, VK_SUBPASS_CONTENTS_INLINE);
}

void VulkanRenderContext::EndRenderPass(RenderPass* _RP)
{
	vkCmdEndRenderPass(CommandBuffers[CurrentImage]);
}

void VulkanRenderContext::BindMesh(Mesh* _Mesh)
{
	const auto l_Mesh = SCAST(VulkanMesh*, _Mesh);
	vkCmdBindVertexBuffers(CommandBuffers[CurrentImage], 0, 1, l_Mesh->GetVertexBuffer(), 0);
	vkCmdBindIndexBuffer(CommandBuffers[CurrentImage], l_Mesh->GetIndexBuffer(), 0, VK_INDEX_TYPE_UINT16);
}

void VulkanRenderContext::BindGraphicsPipeline(GraphicsPipeline* _GP)
{
	vkCmdBindPipeline(CommandBuffers[CurrentImage], VK_PIPELINE_BIND_POINT_GRAPHICS, SCAST(VulkanGraphicsPipeline*, _GP)->GetVk_GraphicsPipeline().GetVkGraphicsPipeline());
}

void VulkanRenderContext::BindComputePipeline(ComputePipeline* _CP)
{
	vkCmdBindPipeline(CommandBuffers[CurrentImage], VK_PIPELINE_BIND_POINT_COMPUTE, SCAST(VulkanComputePipeline*, _CP)->GetVk_ComputePipeline().GetVkPipeline());
}

void VulkanRenderContext::DrawIndexed(const DrawInfo& _DI)
{
	vkCmdDrawIndexed(CommandBuffers[CurrentImage], _DI.IndexCount, _DI.InstanceCount, 0, 0, 0);
}

void VulkanRenderContext::Dispatch(uint32 _WorkGroupsX, uint32 _WorkGroupsY, uint32 _WorkGroupsZ)
{
}


//  VULKAN SWAPCHAIN

Vk_Swapchain::Vk_Swapchain(VkDevice _Device, VkPhysicalDevice _PD, VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent) : VulkanObject(_Device)
{
	FindPresentMode(PresentMode, _PD, _Surface);

	VkSwapchainCreateInfoKHR SwapchainCreateInfo = CreateSwapchainCreateInfo(_Surface, _SurfaceFormat, _SurfaceColorSpace, _SurfaceExtent, PresentMode, VK_NULL_HANDLE);

	auto SwapResult = vkCreateSwapchainKHR(m_Device, &SwapchainCreateInfo, ALLOCATOR, &Swapchain);

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
	vkDestroySwapchainKHR(m_Device, Swapchain, ALLOCATOR);

	VkSwapchainCreateInfoKHR SwapchainCreateInfo = CreateSwapchainCreateInfo(_Surface, _SurfaceFormat, _SurfaceColorSpace, _SurfaceExtent, PresentMode, Swapchain);

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

VkSwapchainCreateInfoKHR Vk_Swapchain::CreateSwapchainCreateInfo(VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode, VkSwapchainKHR _OldSwapchain)
{
	VkSwapchainCreateInfoKHR SwapchainCreateInfo = { VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR };

	SwapchainCreateInfo.surface = _Surface;
	SwapchainCreateInfo.minImageCount = 2;
	SwapchainCreateInfo.imageFormat = _SurfaceFormat;
	SwapchainCreateInfo.imageColorSpace = _SurfaceColorSpace;
	SwapchainCreateInfo.imageExtent = _SurfaceExtent;
	//The imageArrayLayers specifies the amount of layers each image consists of. This is always 1 unless you are developing a stereoscopic 3D application.
	SwapchainCreateInfo.imageArrayLayers = 1;
	//Should be VK_IMAGE_USAGE_TRANSFER_DST_BIT
	SwapchainCreateInfo.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
	SwapchainCreateInfo.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
	SwapchainCreateInfo.queueFamilyIndexCount = 0; // Optional
	SwapchainCreateInfo.pQueueFamilyIndices = nullptr;
	SwapchainCreateInfo.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
	//The compositeAlpha field specifies if the alpha channel should be used for blending with other windows in the window system.
	//You'll almost always want to simply ignore the alpha channel, hence VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR.
	SwapchainCreateInfo.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
	SwapchainCreateInfo.presentMode = _PresentMode;
	SwapchainCreateInfo.clipped = VK_TRUE;
	SwapchainCreateInfo.oldSwapchain = _OldSwapchain;

	return SwapchainCreateInfo;
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

Vk_Surface::Vk_Surface(const Vk_Device& _Device, VkInstance _Instance, VkPhysicalDevice _PD, Window* _Window) : VulkanObject(_Device), m_Instance(_Instance)
{
	VkWin32SurfaceCreateInfoKHR WCreateInfo = { VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR };
	WCreateInfo.hwnd = SCAST(WindowsWindow*, _Window)->GetWindowObject();
	WCreateInfo.hinstance = SCAST(WindowsWindow*, _Window)->GetHInstance();

	GS_VK_CHECK(vkCreateWin32SurfaceKHR(m_Instance, &WCreateInfo, ALLOCATOR, &Surface), "Failed to create Windows Surface!")

	VkSurfaceCapabilitiesKHR Capabilities;
	auto CapResult = vkGetPhysicalDeviceSurfaceCapabilitiesKHR(_PD, Surface, &Capabilities);

	VkBool32 Supports = 0;
	auto SupResult = vkGetPhysicalDeviceSurfaceSupportKHR(_PD, _Device.GetGraphicsQueue().GetQueueIndex(), Surface, &Supports);

	VkSurfaceFormatKHR SF = PickBestFormat(_PD, Surface);
	SurfaceFormat = SF.format;
	SurfaceColorSpace = SF.colorSpace;
}

Vk_Surface::~Vk_Surface()
{
	vkDestroySurfaceKHR(m_Instance, Surface, ALLOCATOR);
}

VkSurfaceFormatKHR Vk_Surface::PickBestFormat(VkPhysicalDevice _PD, VkSurfaceKHR _Surface)
{
	uint32_t FormatsCount = 0;
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, nullptr);
	FVector<VkSurfaceFormatKHR> SurfaceFormats(FormatsCount);
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, SurfaceFormats.data());

	//uint8 i = 0;
	//if (SurfaceFormats[i].colorSpace == VK_COLOR_SPACE_SRGB_NONLINEAR_KHR && SurfaceFormats[i].format == VK_FORMAT_B8G8R8A8_UNORM)
	//{
	//	return SurfaceFormats[i].format;
	//}

	return SurfaceFormats[0];
}
