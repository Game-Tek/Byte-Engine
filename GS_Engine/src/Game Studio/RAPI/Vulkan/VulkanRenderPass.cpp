#include "VulkanRenderPass.h"

#include "Vulkan.h"
#include "Containers/FVector.hpp"
#include "RAPI/Renderer.h"

Tuple<FVector<VkAttachmentDescription>, FVector<VkSubpassDescription>> VulkanRenderPass::CreateInfo(const RenderPassDescriptor& _RPD)
{
	FVector<VkAttachmentDescription> Attachments(1 + _RPD.RenderPassColorAttachments.length());	//Take into account depth/stencil attachment
	//Set color attachments.
	for (uint8 i = 0; i < Attachments.length() - 1; i++) //Loop through all color attachments(skip extra element for depth/stencil)
	{
		Attachments[i].format = FormatToVkFormat(_RPD.RenderPassColorAttachments[i]->GetImageFormat());
		Attachments[i].samples = VK_SAMPLE_COUNT_1_BIT;	//Should match that of the SwapChain images.
		Attachments[i].loadOp = LoadOperationsToVkAttachmentLoadOp(_RPD.RenderPassColorAttachments[i]->GetImageLoadOperation());
		Attachments[i].storeOp = StoreOperationsToVkAttachmentStoreOp(_RPD.RenderPassColorAttachments[i]->GetImageStoreOperation());
		Attachments[i].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
		Attachments[i].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
		Attachments[i].initialLayout = ImageLayoutToVkImageLayout(_RPD.RenderPassColorAttachments[i]->GetImageInitialLayout());
		Attachments[i].finalLayout = ImageLayoutToVkImageLayout(_RPD.RenderPassColorAttachments[i]->GetImageFinalLayout());
	}

	//Set depth/stencil element.
	Attachments[Attachments.length()].format = FormatToVkFormat(_RPD.DepthStencilAttachment->GetImageFormat());
	Attachments[Attachments.length()].samples = VK_SAMPLE_COUNT_1_BIT;
	Attachments[Attachments.length()].loadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
	Attachments[Attachments.length()].storeOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
	Attachments[Attachments.length()].stencilLoadOp = LoadOperationsToVkAttachmentLoadOp(_RPD.DepthStencilAttachment->GetImageLoadOperation());
	Attachments[Attachments.length()].stencilStoreOp = StoreOperationsToVkAttachmentStoreOp(_RPD.DepthStencilAttachment->GetImageStoreOperation());
	Attachments[Attachments.length()].initialLayout = ImageLayoutToVkImageLayout(_RPD.DepthStencilAttachment->GetImageInitialLayout());
	Attachments[Attachments.length()].finalLayout = ImageLayoutToVkImageLayout(_RPD.DepthStencilAttachment->GetImageFinalLayout());



	FVector<FVector<VkAttachmentReference>> SubpassesReferences(_RPD.SubPasses.length());
	for (uint8 SUBPASS = 0; SUBPASS < SubpassesReferences.length(); SUBPASS++)
	{
		FVector<VkAttachmentReference> f(_RPD.SubPasses[SUBPASS].WriteColorAttachments.length() + uint8(1)); //Add element for depth stencil
		SubpassesReferences.push_back(f);
	}
	for (uint8 SUBPASS = 0; SUBPASS < SubpassesReferences.length(); SUBPASS++)
	{
		for (uint8 COLOR_ATTACHMENT = 0; COLOR_ATTACHMENT < SubpassesReferences[SUBPASS].length(); COLOR_ATTACHMENT++)
		{
			SubpassesReferences[SUBPASS][COLOR_ATTACHMENT].attachment = _RPD.SubPasses[SUBPASS].ReadColorAttachments[COLOR_ATTACHMENT].Index;
			SubpassesReferences[SUBPASS][COLOR_ATTACHMENT].layout = ImageLayoutToVkImageLayout(_RPD.SubPasses[SUBPASS].ReadColorAttachments[COLOR_ATTACHMENT].Layout);
		}
	}

	//Describe each subpass.
	FVector<VkSubpassDescription> Subpasses(_RPD.SubPasses.length());
	for (uint8 SUBPASS = 0; SUBPASS < Subpasses.length(); SUBPASS++)	//Loop through each subpass.
	{
		Subpasses[SUBPASS].pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
		Subpasses[SUBPASS].colorAttachmentCount = _RPD.SubPasses[SUBPASS].WriteColorAttachments.length();
		Subpasses[SUBPASS].pColorAttachments = SubpassesReferences[SUBPASS].data();
		Subpasses[SUBPASS].pDepthStencilAttachment = &SubpassesReferences[SUBPASS][SubpassesReferences[SUBPASS].length()];
	}

	return Tuple<FVector<VkAttachmentDescription>, FVector<VkSubpassDescription>>(Attachments, Subpasses);
}

VulkanRenderPass::VulkanRenderPass(const Vk_Device& _Device, const RenderPassDescriptor& _RPD) : RenderPass(_Device, CreateInfo(_RPD))
{



}
