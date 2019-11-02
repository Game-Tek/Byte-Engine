#include "VulkanRenderPass.h"

#include "Vulkan.h"
#include "Containers/FVector.hpp"
#include "RAPI/RenderDevice.h"

VKRenderPassCreator VulkanRenderPass::CreateInfo(VKDevice* _Device, const RenderPassDescriptor& _RPD)
{
	bool DSAA = _RPD.DepthStencilAttachment.AttachmentImage;

	FVector<VkAttachmentDescription> Attachments(_RPD.RenderPassColorAttachments.getLength() + DSAA, VkAttachmentDescription{});	//Take into account depth/stencil attachment
	//Set color attachments.
	for (uint8 i = 0; i < Attachments.getCapacity() - DSAA; i++) //Loop through all color attachments(skip extra element for depth/stencil)
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
		Attachments[Attachments.getCapacity()].format = FormatToVkFormat(_RPD.DepthStencilAttachment.AttachmentImage->GetImageFormat());
		Attachments[Attachments.getCapacity()].samples = VK_SAMPLE_COUNT_1_BIT;
		Attachments[Attachments.getCapacity()].loadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
		Attachments[Attachments.getCapacity()].storeOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
		Attachments[Attachments.getCapacity()].stencilLoadOp = LoadOperationsToVkAttachmentLoadOp(_RPD.DepthStencilAttachment.LoadOperation);
		Attachments[Attachments.getCapacity()].stencilStoreOp = StoreOperationsToVkAttachmentStoreOp(_RPD.DepthStencilAttachment.StoreOperation);
		Attachments[Attachments.getCapacity()].initialLayout = ImageLayoutToVkImageLayout(_RPD.DepthStencilAttachment.InitialLayout);
		Attachments[Attachments.getCapacity()].finalLayout = ImageLayoutToVkImageLayout(_RPD.DepthStencilAttachment.FinalLayout);
	}

	const uint8 AttachmentsCount = _RPD.SubPasses.getLength() * _RPD.RenderPassColorAttachments.getLength();
	DArray<VkAttachmentReference> WriteAttachmentsReferences(AttachmentsCount);
	DArray<VkAttachmentReference> ReadAttachmentsReferences(AttachmentsCount);
	DArray<uint32> PreserveAttachmentsIndices(AttachmentsCount);

	uint8 WriteAttachmentsCount = 0;
	uint8 ReadAttachmentsCount = 0;
	uint8 PreserveAttachmentsCount = 0;

	for (uint8 SUBPASS = 0; SUBPASS < AttachmentsCount; SUBPASS++)
	{
		for (uint8 ATT = 0; ATT < _RPD.RenderPassColorAttachments.getLength(); ATT++)
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

		for (uint8 ATT = 0; ATT < _RPD.RenderPassColorAttachments.getLength(); ATT++)
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

		for (uint8 ATT = 0; ATT < _RPD.RenderPassColorAttachments.getLength(); ATT++)
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
	FVector<VkSubpassDescription> Subpasses(_RPD.SubPasses.getLength(), VkSubpassDescription{});
	for (uint8 SUBPASS = 0; SUBPASS < Subpasses.getLength(); SUBPASS++)	//Loop through each subpass.
	{
		Subpasses[SUBPASS].pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
		Subpasses[SUBPASS].colorAttachmentCount = WriteAttachmentsCount;
		Subpasses[SUBPASS].pColorAttachments = WriteAttachmentsReferences.getData() + SUBPASS;
		Subpasses[SUBPASS].inputAttachmentCount = ReadAttachmentsCount;
		Subpasses[SUBPASS].pInputAttachments = ReadAttachmentsReferences.getData() + SUBPASS;
		Subpasses[SUBPASS].pResolveAttachments = nullptr;
		Subpasses[SUBPASS].preserveAttachmentCount = 0;//PreserveAttachmentsCount;
		Subpasses[SUBPASS].pPreserveAttachments = PreserveAttachmentsIndices.getData() + SUBPASS;
		Subpasses[SUBPASS].pDepthStencilAttachment = DSAA ? &WriteAttachmentsReferences[SUBPASS] : nullptr;
	}


	uint8 ArrayLength = 0;
	for (uint8 i = 0; i < _RPD.SubPasses.getLength(); ++i)
	{
		ArrayLength += _RPD.SubPasses[i]->ReadColorAttachments.getLength() + _RPD.SubPasses[i]->WriteColorAttachments.getLength();
	}

	DArray<VkSubpassDependency> SubpassDependencies(ArrayLength);
	for (uint8 SUBPASS = 0; SUBPASS < _RPD.SubPasses.getLength(); ++SUBPASS)
	{
		for (uint8 ATT = 0; ATT < _RPD.SubPasses[SUBPASS]->ReadColorAttachments.getLength() + _RPD.SubPasses[SUBPASS]->WriteColorAttachments.getLength(); ++ATT)
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
	RPCI.attachmentCount = _RPD.RenderPassColorAttachments.getLength() + DSAA;
	RPCI.pAttachments = Attachments.getData();
	RPCI.subpassCount = _RPD.SubPasses.getLength();
	RPCI.pSubpasses = Subpasses.getData();
	RPCI.dependencyCount = ArrayLength;
	RPCI.pDependencies = SubpassDependencies.getData();

	return VKRenderPassCreator(_Device, &RPCI);
}

VulkanRenderPass::VulkanRenderPass(VKDevice* _Device, const RenderPassDescriptor& _RPD) : RenderPass(CreateInfo(_Device, _RPD))
{



}
