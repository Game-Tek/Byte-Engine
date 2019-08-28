#include "Vulkan.h"

#include "VulkanRenderer.h"

#include "VulkanRenderContext.h"
#include "VulkanRenderPass.h"
#include "VulkanFramebuffer.h"
#include "VulkanPipelines.h"
#include "VulkanMesh.h"

#include "RAPI/Window.h"
#include "Native/Vk_PhysicalDevice.h"


//  VULKAN RENDER CONTEXT

uint8 ScorePresentMode(VkPresentModeKHR _PresentMode)
{
	switch (_PresentMode)
	{
	case VK_PRESENT_MODE_MAILBOX_KHR:	return 255;
	case VK_PRESENT_MODE_FIFO_KHR:		return 254;
	default:							return 0;
	}
}


SurfaceFormat VulkanRenderContext::FindFormat(const Vk_PhysicalDevice& _PD, VkSurfaceKHR _Surface)
{
	uint32_t FormatsCount = 0;
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, nullptr);
	DArray<VkSurfaceFormatKHR> SurfaceFormats(FormatsCount);
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, SurfaceFormats.data());

	return { SurfaceFormats[0].format, SurfaceFormats[0].colorSpace };
}

VkPresentModeKHR VulkanRenderContext::FindPresentMode(const Vk_PhysicalDevice& _PD, const Vk_Surface& _Surface)
{
	uint32_t PresentModesCount = 0;
	vkGetPhysicalDeviceSurfacePresentModesKHR(_PD, _Surface, &PresentModesCount, nullptr);
	DArray<VkPresentModeKHR> PresentModes(PresentModesCount);
	vkGetPhysicalDeviceSurfacePresentModesKHR(_PD, _Surface, &PresentModesCount, PresentModes.data());

	uint8 BestScore = 0;
	uint8 BestPresentModeIndex = 0;
	for (uint8 i = 0; i < PresentModes.length(); i++)
	{
		if (ScorePresentMode(PresentModes[i]) > BestScore)
		{
			BestScore = ScorePresentMode(PresentModes[i]);

			BestPresentModeIndex = i;
		}
	}

	return PresentModes[BestPresentModeIndex];
}

VulkanRenderContext::VulkanRenderContext(const Vk_Device& _Device, const Vk_Instance& _Instance, const Vk_PhysicalDevice& _PD, const Window& _Window) :
	RenderExtent(_Window.GetWindowExtent()),
	Surface(_Device, _Instance, _PD, _Window),
	Format(FindFormat(_PD, Surface)),
	PresentMode(FindPresentMode(_PD, Surface)),
	Swapchain(_Device, Surface, Format.format, Format.colorSpace, Extent2DToVkExtent2D(RenderExtent), PresentMode),
	SwapchainImages(Swapchain.GetImages()),
	Images(SwapchainImages.length()),
	ImagesAvailable(SwapchainImages.length()),
	RendersFinished(SwapchainImages.length()),
	InFlightFences(SwapchainImages.length()),
	PresentationQueue(_Device.GetGraphicsQueue()),
	CommandPool(_Device, _Device.GetGraphicsQueue(), VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT),
	CommandBuffers(SwapchainImages.length())
{
	MAX_FRAMES_IN_FLIGHT = SCAST(uint8, SwapchainImages.length());

	for (uint8 i = 0; i < MAX_FRAMES_IN_FLIGHT; ++i)
	{
		ImagesAvailable.push_back(new Vk_Semaphore(_Device));
		RendersFinished.push_back(new Vk_Semaphore(_Device));
		InFlightFences.push_back(new Vk_Fence(_Device, true));
		CommandBuffers.push_back(new Vk_CommandBuffer(_Device, CommandPool));

		Images.push_back(new VulkanSwapchainImage(_Device, SwapchainImages[i], VkFormatToFormat(Format.format)));
	}
}

VulkanRenderContext::~VulkanRenderContext()
{
	for (uint8 i = 0; i < MAX_FRAMES_IN_FLIGHT; ++i)
	{
		delete ImagesAvailable[i];
		delete RendersFinished[i];
		delete InFlightFences[i];
		delete CommandBuffers[i];
	}

	FVector<VulkanSwapchainImage*>::DestroyFVectorOfPointers(Images);
}

void VulkanRenderContext::OnResize()
{
}

void VulkanRenderContext::AcquireNextImage()
{
	const auto lImageIndex = Swapchain.AcquireNextImage(*ImagesAvailable[CurrentImage]); //This signals the semaphore when the image becomes available
	ImageIndex = lImageIndex;
}

void VulkanRenderContext::Flush()
{
	InFlightFences[CurrentImage]->Wait();	//Get current's frame fences and wait for it.
	InFlightFences[CurrentImage]->Reset();	//Then reset it.

	VkPipelineStageFlags WaitStages[] = { VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT };

	VkSemaphore WaitSemaphores[] = { *ImagesAvailable[CurrentImage] };		//Set current's frame ImageAvaiable semaphore as the semaphore to wait for to start rendering to.
	VkSemaphore SignalSemaphores[] = { *RendersFinished[CurrentImage] };	//Set current's frame RenderFinished semaphore as the semaphore to signal once rendering has finished.
	VkCommandBuffer lCommandBuffers[] = { *CommandBuffers[CurrentImage] };	

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

	PresentationQueue.Submit(&SubmitInfo, *InFlightFences[CurrentImage]);	//Signal fence when execution of this queue has finished.

	InFlightFences[CurrentImage]->Wait();
	CommandBuffers[CurrentImage]->Reset();
}

void VulkanRenderContext::Present()
{
	VkSemaphore WaitSemaphores[] = { *RendersFinished[CurrentImage] };

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
	
	CurrentImage = (CurrentImage + 1) % MAX_FRAMES_IN_FLIGHT;
}

void VulkanRenderContext::BeginRecording()
{
	VkCommandBufferBeginInfo BeginInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
	BeginInfo.flags = VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT;
	//Hint to primary buffer if this is secondary.
	BeginInfo.pInheritanceInfo = nullptr;

	CommandBuffers[CurrentImage]->Begin(&BeginInfo);
}

void VulkanRenderContext::EndRecording()
{
	CommandBuffers[CurrentImage]->End();
}

void VulkanRenderContext::BeginRenderPass(const RenderPassBeginInfo& _RPBI)
{
	VkClearValue ClearColor = { 0.0f, 0.0f, 0.0f, 1.0f };

	VkRenderPassBeginInfo RenderPassBeginInfo = { VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO };
	RenderPassBeginInfo.renderPass = SCAST(VulkanRenderPass*, _RPBI.RenderPass)->GetVk_RenderPass();
	RenderPassBeginInfo.pClearValues = &ClearColor;
	RenderPassBeginInfo.clearValueCount = 1;
	RenderPassBeginInfo.framebuffer = SCAST(VulkanFramebuffer*, _RPBI.Framebuffers[CurrentImage])->GetVk_Framebuffer();
	RenderPassBeginInfo.renderArea.extent = Extent2DToVkExtent2D(RenderExtent);
	RenderPassBeginInfo.renderArea.offset = { 0, 0 };

	vkCmdBeginRenderPass(*CommandBuffers[CurrentImage], &RenderPassBeginInfo, VK_SUBPASS_CONTENTS_INLINE);
}

void VulkanRenderContext::AdvanceSubPass()
{
	vkCmdNextSubpass(*CommandBuffers[CurrentImage], VK_SUBPASS_CONTENTS_INLINE);
}

void VulkanRenderContext::EndRenderPass(RenderPass* _RP)
{
	vkCmdEndRenderPass(*CommandBuffers[CurrentImage]);
}

void VulkanRenderContext::BindMesh(Mesh* _Mesh)
{
	const auto l_Mesh = SCAST(VulkanMesh*, _Mesh);
	VkDeviceSize Offset = 0;
	vkCmdBindVertexBuffers(*CommandBuffers[CurrentImage], 0, 1, l_Mesh->GetVertexBuffer(), &Offset);
	vkCmdBindIndexBuffer(*CommandBuffers[CurrentImage], l_Mesh->GetIndexBuffer(), 0, VK_INDEX_TYPE_UINT16);
}

void VulkanRenderContext::BindGraphicsPipeline(GraphicsPipeline* _GP)
{
	vkCmdBindPipeline(*CommandBuffers[CurrentImage], VK_PIPELINE_BIND_POINT_GRAPHICS, SCAST(VulkanGraphicsPipeline*, _GP)->GetVk_GraphicsPipeline());
}

void VulkanRenderContext::BindComputePipeline(ComputePipeline* _CP)
{
	vkCmdBindPipeline(*CommandBuffers[CurrentImage], VK_PIPELINE_BIND_POINT_COMPUTE, SCAST(VulkanComputePipeline*, _CP)->GetVk_ComputePipeline());
}

void VulkanRenderContext::DrawIndexed(const DrawInfo& _DI)
{
	vkCmdDrawIndexed(*CommandBuffers[CurrentImage], _DI.IndexCount, _DI.InstanceCount, 0, 0, 0);
}

void VulkanRenderContext::Dispatch(const Extent3D& _WorkGroups)
{
	vkCmdDispatch(*CommandBuffers[CurrentImage], _WorkGroups.Width, _WorkGroups.Height, _WorkGroups.Depth);
}

FVector<Image*> VulkanRenderContext::GetSwapchainImages() const
{
	FVector<Image*> l_Images(MAX_FRAMES_IN_FLIGHT);
	for (uint8 i = 0; i < MAX_FRAMES_IN_FLIGHT; ++i)
	{
		l_Images.push_back(Images[i]);
	}

	return l_Images;
}
