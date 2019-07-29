#include "VulkanRenderPass.h"

#include "Vulkan.h"
#include "Containers/FVector.hpp"
#include "RAPI/Renderer.h"
#include "VulkanRenderer.h"

VulkanRenderPass::VulkanRenderPass(VkDevice _Device, const RenderPassDescriptor& _RPD) : RenderPass(_Device, _RPD)
{
	FVector<VkAttachmentDescription> Attachments(1 + _RPD.ColorAttachmentsCount);	//Take into account depth/stencil attachment
//Set color attachments.
	for (uint8 i = 0; i < Attachments.length() - 1; i++) //Loop through all color attachments(skip extra element for depth/stencil)
	{
		Attachments[i].format = FormatToVkFormat(_RPD.ColorAttachments[0].AttachmentFormat);
		Attachments[i].samples = VK_SAMPLE_COUNT_1_BIT;	//Should match that of the SwapChain images.
		Attachments[i].loadOp = LoadOperationsToVkAttachmentLoadOp(_RPD.ColorAttachments[i].LoadOperation);
		Attachments[i].storeOp = StoreOperationsToVkAttachmentStoreOp(_RPD.ColorAttachments[i].StoreOperation);
		Attachments[i].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
		Attachments[i].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
		Attachments[i].initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
		Attachments[i].finalLayout = ImageLayoutToVkImageLayout(_RPD.ColorAttachments[0].Layout);
	}

	//Set depth/stencil element.
	Attachments[Attachments.length()].format = FormatToVkFormat(_RPD.DepthStencilAttachment.AttachmentFormat);
	Attachments[Attachments.length()].samples = VK_SAMPLE_COUNT_1_BIT;
	Attachments[Attachments.length()].loadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
	Attachments[Attachments.length()].storeOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
	Attachments[Attachments.length()].stencilLoadOp = LoadOperationsToVkAttachmentLoadOp(_RPD.DepthStencilAttachment.LoadOperation);
	Attachments[Attachments.length()].stencilStoreOp = StoreOperationsToVkAttachmentStoreOp(_RPD.DepthStencilAttachment.StoreOperation);
	Attachments[Attachments.length()].initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
	Attachments[Attachments.length()].finalLayout = ImageLayoutToVkImageLayout(_RPD.DepthStencilAttachment.Layout);


	FVector<FVector<VkAttachmentReference>> SubpassesReferences(_RPD.SubPassesCount);
	for (uint8 SUBPASS = 0; SUBPASS < SubpassesReferences.length(); SUBPASS++)
	{
		FVector<VkAttachmentReference> f(_RPD.SubPasses[SUBPASS].ColorAttachmentsCount + uint8(1)); //Add element for depth stencil
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
	FVector<VkSubpassDescription> Subpasses(_RPD.SubPassesCount);
	for (uint8 SUBPASS = 0; SUBPASS < Subpasses.length(); SUBPASS++)	//Loop through each subpass.
	{
		Subpasses[SUBPASS].pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
		Subpasses[SUBPASS].colorAttachmentCount = _RPD.SubPasses[SUBPASS].ColorAttachmentsCount;
		Subpasses[SUBPASS].pColorAttachments = SubpassesReferences[SUBPASS].data();
		Subpasses[SUBPASS].pDepthStencilAttachment = &SubpassesReferences[SUBPASS][SubpassesReferences[SUBPASS].length()];
	}



}