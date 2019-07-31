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
	FVector<VkSurfaceFormatKHR> SurfaceFormats(FormatsCount);
	vkGetPhysicalDeviceSurfaceFormatsKHR(_PD, _Surface, &FormatsCount, SurfaceFormats.data());

	//uint8 i = 0;
	//if (SurfaceFormats[i].colorSpace == VK_COLOR_SPACE_SRGB_NONLINEAR_KHR && SurfaceFormats[i].format == VK_FORMAT_B8G8R8A8_UNORM)
	//{
	//	return SurfaceFormats[i].format;
	//}

	return { SurfaceFormats[0].format, SurfaceFormats[0].colorSpace };
}

VkPresentModeKHR VulkanRenderContext::FindPresentMode(const Vk_PhysicalDevice& _PD, const Vk_Surface& _Surface)
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

	return PresentModes[BestPresentModeIndex];
}

VulkanRenderContext::VulkanRenderContext(const Vk_Device& _Device, const Vk_Instance& _Instance, const Vk_PhysicalDevice& _PD, const Window& _Window) :
	Surface(_Device, _Instance, _PD, _Window),
	Format(FindFormat(_PD, Surface)),
	PresentMode(FindPresentMode(_PD, Surface)),
	Swapchain(_Device, Surface, Format.format, Format.colorSpace, Extent2DToVkExtent2D(_Window.GetWindowExtent()), PresentMode),
	ImageAvailable(_Device),
	RenderFinished(_Device),
	PresentationQueue(_Device.GetGraphicsQueue()),
	CommandPool(_Device, _Device.GetGraphicsQueue()),
	MaxFramesInFlight(Swapchain.GetImages().length()),
	CommandBuffers(MaxFramesInFlight, Vk_CommandBuffer(_Device, CommandPool))
{
}

void VulkanRenderContext::OnResize()
{
}

void VulkanRenderContext::Present()
{
	VkSemaphore WaitSemaphores[] = { ImageAvailable };

	/* Present result on screen */
	const VkSwapchainKHR Swapchains[] = { Swapchain.GetVkSwapchain() };

	const uint32 ImageIndex = Swapchain.AcquireNextImage(ImageAvailable);
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
	VkSemaphore WaitSemaphores[] = { ImageAvailable };
	VkSemaphore SignalSemaphores[] = { RenderFinished };
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
	RenderPassBeginInfo.renderPass = SCAST(VulkanRenderPass*, _RPBI.RenderPass)->GetVk_RenderPass();
	RenderPassBeginInfo.pClearValues = &ClearColor;
	RenderPassBeginInfo.clearValueCount = 1;
	RenderPassBeginInfo.framebuffer = SCAST(VulkanFramebuffer*, _RPBI.Framebuffer)->GetVk_Framebuffer();
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