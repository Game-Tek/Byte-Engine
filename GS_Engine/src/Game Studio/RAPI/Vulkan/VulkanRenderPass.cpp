#include "VulkanRenderPass.h"

#include "Vulkan.h"
#include "Containers/FVector.hpp"
#include "RAPI/Renderer.h"

Vk_RenderPassCreateInfo VulkanRenderPass::CreateInfo(const Vk_Device& _Device, const RenderPassDescriptor& _RPD)
{
	bool DSAA = _RPD.DepthStencilAttachment.AttachmentImage;

	FVector<VkAttachmentDescription> Attachments(_RPD.RenderPassColorAttachments.length() + DSAA, VkAttachmentDescription{});	//Take into account depth/stencil attachment
	//Set color attachments.
	for (uint8 i = 0; i < Attachments.capacity() - DSAA; i++) //Loop through all color attachments(skip extra element for depth/stencil)
	{
		Attachments[i].format = FormatToVkFormat(_RPD.RenderPassColorAttachments[i]->AttachmentImage->GetImageFormat());
		Attachments[i].samples = VK_SAMPLE_COUNT_1_BIT;	//Should match that of the SwapChain images.
		Attachments[i].loadOp = LoadOperationsToVkAttachmentLoadOp(_RPD.RenderPassColorAttachments[i]->LoadOperation);
		Attachments[i].storeOp = StoreOperationsToVkAttachmentStoreOp(_RPD.RenderPassColorAttachments[i]->StoreOperation);
		Attachments[i].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
		Attachments[i].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
		Attachments[i].initialLayout = ImageLayoutToVkImageLayout(_RPD.RenderPassColorAttachments[i]->InitialLayout);
		Attachments[i].finalLayout = ImageLayoutToVkImageLayout(_RPD.RenderPassColorAttachments[i]->FinalLayout);
	}

	if (DSAA)
	{
		//Set depth/stencil element.
		Attachments[Attachments.capacity()].format = FormatToVkFormat(_RPD.DepthStencilAttachment.AttachmentImage->GetImageFormat());
		Attachments[Attachments.capacity()].samples = VK_SAMPLE_COUNT_1_BIT;
		Attachments[Attachments.capacity()].loadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
		Attachments[Attachments.capacity()].storeOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
		Attachments[Attachments.capacity()].stencilLoadOp = LoadOperationsToVkAttachmentLoadOp(_RPD.DepthStencilAttachment.LoadOperation);
		Attachments[Attachments.capacity()].stencilStoreOp = StoreOperationsToVkAttachmentStoreOp(_RPD.DepthStencilAttachment.StoreOperation);
		Attachments[Attachments.capacity()].initialLayout = ImageLayoutToVkImageLayout(_RPD.DepthStencilAttachment.InitialLayout);
		Attachments[Attachments.capacity()].finalLayout = ImageLayoutToVkImageLayout(_RPD.DepthStencilAttachment.FinalLayout);
	}

	const uint8 AttachmentsCount = _RPD.SubPasses.length() * _RPD.RenderPassColorAttachments.length();
	DArray<VkAttachmentReference> WriteAttachmentsReferences(AttachmentsCount);
	DArray<VkAttachmentReference> ReadAttachmentsReferences(AttachmentsCount);
	DArray<uint32> PreserveAttachmentsIndices(AttachmentsCount);

	uint8 WriteAttachmentsCount = 0;
	uint8 ReadAttachmentsCount = 0;
	uint8 PreserveAttachmentsCount = 0;

	for (uint8 SUBPASS = 0; SUBPASS < AttachmentsCount; SUBPASS++)
	{
		for (uint8 ATT = 0; ATT < _RPD.RenderPassColorAttachments.length(); ATT++)
		{
			if (_RPD.SubPasses[SUBPASS]->WriteColorAttachments[ATT]->Index == ATTACHMENT_UNUSED)
			{
				WriteAttachmentsReferences[SUBPASS + ATT].attachment = VK_ATTACHMENT_UNUSED;
				WriteAttachmentsReferences[SUBPASS + ATT].layout = VK_IMAGE_LAYOUT_UNDEFINED;
			}
			else
			{
				WriteAttachmentsReferences[SUBPASS + ATT].attachment = _RPD.SubPasses[SUBPASS]->WriteColorAttachments[ATT]->Index;
				WriteAttachmentsReferences[SUBPASS + ATT].layout = ImageLayoutToVkImageLayout(_RPD.SubPasses[SUBPASS]->WriteColorAttachments[ATT]->Layout);

				WriteAttachmentsCount++;
			}
		}

		for (uint8 ATT = 0; ATT < _RPD.RenderPassColorAttachments.length(); ATT++)
		{
			if(_RPD.SubPasses[SUBPASS]->ReadColorAttachments[ATT]->Index == ATTACHMENT_UNUSED)
			{
				ReadAttachmentsReferences[SUBPASS + ATT].attachment = VK_ATTACHMENT_UNUSED;
				ReadAttachmentsReferences[SUBPASS + ATT].layout = VK_IMAGE_LAYOUT_UNDEFINED;
			}
			else
			{
				ReadAttachmentsReferences[SUBPASS + ATT].attachment = _RPD.SubPasses[SUBPASS]->ReadColorAttachments[ATT]->Index;
				ReadAttachmentsReferences[SUBPASS + ATT].layout = ImageLayoutToVkImageLayout(_RPD.SubPasses[SUBPASS]->ReadColorAttachments[ATT]->Layout);

				ReadAttachmentsCount++;
			}
		}

		for (uint8 ATT = 0; ATT < _RPD.RenderPassColorAttachments.length(); ATT++)
		{
			if(_RPD.SubPasses[SUBPASS]->PreserveAttachments[ATT] == ATTACHMENT_UNUSED)
			{
				PreserveAttachmentsIndices[SUBPASS + ATT] = VK_ATTACHMENT_UNUSED;
			}
			else
			{
				PreserveAttachmentsIndices[SUBPASS + ATT] = _RPD.SubPasses[SUBPASS]->PreserveAttachments[ATT];

				PreserveAttachmentsCount++;
			}
		}
	}

	//Describe each subpass.
	FVector<VkSubpassDescription> Subpasses(_RPD.SubPasses.length(), VkSubpassDescription{});
	for (uint8 SUBPASS = 0; SUBPASS < Subpasses.length(); SUBPASS++)	//Loop through each subpass.
	{
		Subpasses[SUBPASS].pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
		Subpasses[SUBPASS].colorAttachmentCount = WriteAttachmentsCount;
		Subpasses[SUBPASS].pColorAttachments = WriteAttachmentsReferences.data() + SUBPASS;
		Subpasses[SUBPASS].inputAttachmentCount = ReadAttachmentsCount;
		Subpasses[SUBPASS].pInputAttachments = ReadAttachmentsReferences.data() + SUBPASS;
		Subpasses[SUBPASS].pResolveAttachments = nullptr;
		Subpasses[SUBPASS].preserveAttachmentCount = 0;//PreserveAttachmentsCount;
		Subpasses[SUBPASS].pPreserveAttachments = PreserveAttachmentsIndices.data() + SUBPASS;
		Subpasses[SUBPASS].pDepthStencilAttachment = DSAA ? &WriteAttachmentsReferences[SUBPASS] : nullptr;
	}


	uint8 ArrayLength = 0;
	for (uint8 i = 0; i < _RPD.SubPasses.length(); ++i)
	{
		ArrayLength += _RPD.SubPasses[i]->ReadColorAttachments.length() + _RPD.SubPasses[i]->WriteColorAttachments.length();
	}

	DArray<VkSubpassDependency> SubpassDependencies(ArrayLength);
	for (uint8 SUBPASS = 0; SUBPASS < _RPD.SubPasses.length(); ++SUBPASS)
	{
		for (uint8 ATT = 0; ATT < _RPD.SubPasses[SUBPASS]->ReadColorAttachments.length() + _RPD.SubPasses[SUBPASS]->WriteColorAttachments.length(); ++ATT)
		{
			SubpassDependencies[SUBPASS + ATT].srcSubpass = VK_SUBPASS_EXTERNAL;
			SubpassDependencies[SUBPASS + ATT].dstSubpass = SUBPASS;
			SubpassDependencies[SUBPASS + ATT].srcStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
			SubpassDependencies[SUBPASS + ATT].srcAccessMask = 0;
			SubpassDependencies[SUBPASS + ATT].dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
			SubpassDependencies[SUBPASS + ATT].dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
			SubpassDependencies[SUBPASS + ATT].dependencyFlags = 0;
		}
	}

	VkRenderPassCreateInfo RPCI = { VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO };
	RPCI.attachmentCount = _RPD.RenderPassColorAttachments.length() + DSAA;
	RPCI.pAttachments = Attachments.data();
	RPCI.subpassCount = _RPD.SubPasses.length();
	RPCI.pSubpasses = Subpasses.data();
	RPCI.dependencyCount = ArrayLength;
	RPCI.pDependencies = SubpassDependencies.data();

	return Vk_RenderPass::CreateVk_RenderPassCreateInfo(_Device, &RPCI);
}

VulkanRenderPass::VulkanRenderPass(const Vk_Device& _Device, const RenderPassDescriptor& _RPD) : RenderPass(CreateInfo(_Device, _RPD))
{



}
