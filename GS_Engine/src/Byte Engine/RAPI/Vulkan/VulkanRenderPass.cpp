#include "VulkanRenderPass.h"

#include "Containers/FVector.hpp"
#include "RAPI/Vulkan/VulkanRenderDevice.h"

VulkanRenderPass::VulkanRenderPass(class VulkanRenderDevice * vulkanRenderDevice, const RAPI::RenderPassCreateInfo& renderPassCreateInfo)
{
	bool DSAA = renderPassCreateInfo.Descriptor.DepthStencilAttachment.AttachmentImage;

	FVector<VkAttachmentDescription> Attachments(renderPassCreateInfo.Descriptor.RenderPassColorAttachments.getLength() + DSAA, renderPassCreateInfo.Descriptor.RenderPassColorAttachments.getLength() + DSAA);
	//Take into account depth/stencil attachment
	{
		//Set color attachments.
		for (uint8 i = 0; i < Attachments.getCapacity() - DSAA; i++)
			//Loop through all color attachments(skip extra element for depth/stencil)
		{
			Attachments[i].format = FormatToVkFormat(renderPassCreateInfo.Descriptor.RenderPassColorAttachments[i]->AttachmentImage->GetFormat());
			Attachments[i].samples = VK_SAMPLE_COUNT_1_BIT; //Should match that of the SwapChain images.
			Attachments[i].loadOp = RenderTargetLoadOperationsToVkAttachmentLoadOp(renderPassCreateInfo.Descriptor.RenderPassColorAttachments[i]->LoadOperation);
			Attachments[i].storeOp = RenderTargetStoreOperationsToVkAttachmentStoreOp(renderPassCreateInfo.Descriptor.RenderPassColorAttachments[i]->StoreOperation);
			Attachments[i].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
			Attachments[i].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
			Attachments[i].initialLayout = ImageLayoutToVkImageLayout(renderPassCreateInfo.Descriptor.RenderPassColorAttachments[i]->InitialLayout);
			Attachments[i].finalLayout = ImageLayoutToVkImageLayout(renderPassCreateInfo.Descriptor.RenderPassColorAttachments[i]->FinalLayout);
		}

		if (DSAA)
		{
			//Set depth/stencil element.
			Attachments[Attachments.getCapacity() - 1].format = FormatToVkFormat(renderPassCreateInfo.Descriptor.DepthStencilAttachment.AttachmentImage->GetFormat());
			Attachments[Attachments.getCapacity() - 1].samples = VK_SAMPLE_COUNT_1_BIT;
			Attachments[Attachments.getCapacity() - 1].loadOp = RenderTargetLoadOperationsToVkAttachmentLoadOp(renderPassCreateInfo.Descriptor.DepthStencilAttachment.LoadOperation);
			Attachments[Attachments.getCapacity() - 1].storeOp = RenderTargetStoreOperationsToVkAttachmentStoreOp(renderPassCreateInfo.Descriptor.DepthStencilAttachment.StoreOperation);
			Attachments[Attachments.getCapacity() - 1].stencilLoadOp = RenderTargetLoadOperationsToVkAttachmentLoadOp(renderPassCreateInfo.Descriptor.DepthStencilAttachment.LoadOperation);
			Attachments[Attachments.getCapacity() - 1].stencilStoreOp = RenderTargetStoreOperationsToVkAttachmentStoreOp(renderPassCreateInfo.Descriptor.DepthStencilAttachment.StoreOperation);
			Attachments[Attachments.getCapacity() - 1].initialLayout = ImageLayoutToVkImageLayout(renderPassCreateInfo.Descriptor.DepthStencilAttachment.InitialLayout);
			Attachments[Attachments.getCapacity() - 1].finalLayout = ImageLayoutToVkImageLayout(renderPassCreateInfo.Descriptor.DepthStencilAttachment.FinalLayout);
		}
	}

	const uint8 attachments_count = renderPassCreateInfo.Descriptor.SubPasses.getLength() * renderPassCreateInfo.Descriptor.RenderPassColorAttachments.getLength();
	DArray<VkAttachmentReference> WriteAttachmentsReferences(attachments_count);
	DArray<VkAttachmentReference> ReadAttachmentsReferences(attachments_count);
	DArray<uint32> PreserveAttachmentsIndices(attachments_count);
	DArray<VkAttachmentReference> depth_attachment_references(renderPassCreateInfo.Descriptor.SubPasses.getLength());

	uint8 WriteAttachmentsCount = 0;
	uint8 ReadAttachmentsCount = 0;
	uint8 PreserveAttachmentsCount = 0;

	for (uint8 SUBPASS = 0; SUBPASS < renderPassCreateInfo.Descriptor.SubPasses.getLength(); ++SUBPASS)
	{
		uint8 written_attachment_references_this_subpass_loop = 0;

		for (uint8 ATT = 0; ATT < renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->WriteColorAttachments.getLength(); ++ATT)
		{
			if (renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->WriteColorAttachments[ATT]->Index == ATTACHMENT_UNUSED)
			{
				WriteAttachmentsReferences[SUBPASS + ATT].attachment = VK_ATTACHMENT_UNUSED;
				WriteAttachmentsReferences[SUBPASS + ATT].layout = VK_IMAGE_LAYOUT_UNDEFINED;
			}
			else
			{
				WriteAttachmentsReferences[SUBPASS + ATT].attachment = renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->WriteColorAttachments[ATT]->Index;
				WriteAttachmentsReferences[SUBPASS + ATT].layout = ImageLayoutToVkImageLayout(renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->WriteColorAttachments[ATT]->Layout);

				++WriteAttachmentsCount;
				++written_attachment_references_this_subpass_loop;
			}
		}

		for (uint8 ATT = 0; ATT < renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->ReadColorAttachments.getLength(); ++ATT)
		{
			if (renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->ReadColorAttachments[ATT]->Index == ATTACHMENT_UNUSED)
			{
				ReadAttachmentsReferences[SUBPASS + ATT].attachment = VK_ATTACHMENT_UNUSED;
				ReadAttachmentsReferences[SUBPASS + ATT].layout = VK_IMAGE_LAYOUT_UNDEFINED;
			}
			else
			{
				ReadAttachmentsReferences[SUBPASS + ATT].attachment = renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->ReadColorAttachments[ATT]->Index;
				ReadAttachmentsReferences[SUBPASS + ATT].layout = ImageLayoutToVkImageLayout(renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->ReadColorAttachments[ATT]->Layout);

				++ReadAttachmentsCount;
				++written_attachment_references_this_subpass_loop;
			}
		}

		for (uint8 ATT = 0; ATT < renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->PreserveAttachments.getLength(); ++ATT)
		{
			if (renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->PreserveAttachments[ATT] == ATTACHMENT_UNUSED)
			{
				PreserveAttachmentsIndices[SUBPASS + ATT] = VK_ATTACHMENT_UNUSED;
			}
			else
			{
				PreserveAttachmentsIndices[SUBPASS + ATT] = renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->PreserveAttachments[ATT];

				PreserveAttachmentsCount++;
				++written_attachment_references_this_subpass_loop;
			}
		}

		if (renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->DepthAttachmentReference)
		{
			depth_attachment_references[SUBPASS].attachment = renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->DepthAttachmentReference->Index;
			depth_attachment_references[SUBPASS].layout = ImageLayoutToVkImageLayout(renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->DepthAttachmentReference->Layout);
		}
		else
		{
			depth_attachment_references[SUBPASS].attachment = VK_ATTACHMENT_UNUSED;
			depth_attachment_references[SUBPASS].layout = VK_IMAGE_LAYOUT_UNDEFINED;
		}
	}

	//Describe each subpass.
	FVector<VkSubpassDescription> Subpasses(renderPassCreateInfo.Descriptor.SubPasses.getLength(), renderPassCreateInfo.Descriptor.SubPasses.getLength());
	for (uint8 SUBPASS = 0; SUBPASS < Subpasses.getLength(); SUBPASS++) //Loop through each subpass.
	{
		Subpasses[SUBPASS].pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
		Subpasses[SUBPASS].colorAttachmentCount = WriteAttachmentsCount;
		Subpasses[SUBPASS].pColorAttachments = WriteAttachmentsReferences.getData() + SUBPASS;
		Subpasses[SUBPASS].inputAttachmentCount = ReadAttachmentsCount;
		Subpasses[SUBPASS].pInputAttachments = ReadAttachmentsReferences.getData() + SUBPASS;
		Subpasses[SUBPASS].pResolveAttachments = nullptr;
		Subpasses[SUBPASS].preserveAttachmentCount = 0; //PreserveAttachmentsCount;
		Subpasses[SUBPASS].pPreserveAttachments = PreserveAttachmentsIndices.getData() + SUBPASS;
		Subpasses[SUBPASS].pDepthStencilAttachment = &depth_attachment_references[SUBPASS];
	}

	uint8 ArrayLength = 0;
	for (uint8 i = 0; i < renderPassCreateInfo.Descriptor.SubPasses.getLength(); ++i)
	{
		ArrayLength += renderPassCreateInfo.Descriptor.SubPasses[i]->ReadColorAttachments.getLength() + renderPassCreateInfo.Descriptor.SubPasses[i]->WriteColorAttachments.getLength();
	}

	DArray<VkSubpassDependency> SubpassDependencies(ArrayLength);
	for (uint8 SUBPASS = 0; SUBPASS < renderPassCreateInfo.Descriptor.SubPasses.getLength(); ++SUBPASS)
	{
		for (uint8 ATT = 0; ATT < renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->ReadColorAttachments.getLength() + renderPassCreateInfo.Descriptor.SubPasses[SUBPASS]->WriteColorAttachments.getLength(); ++ATT)
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

	VkRenderPassCreateInfo vk_renderpass_create_info{ VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO };
	vk_renderpass_create_info.attachmentCount = renderPassCreateInfo.Descriptor.RenderPassColorAttachments.getLength() + DSAA;
	vk_renderpass_create_info.pAttachments = Attachments.getData();
	vk_renderpass_create_info.subpassCount = renderPassCreateInfo.Descriptor.SubPasses.getLength();
	vk_renderpass_create_info.pSubpasses = Subpasses.getData();
	vk_renderpass_create_info.dependencyCount = ArrayLength;
	vk_renderpass_create_info.pDependencies = SubpassDependencies.getData();

	vkCreateRenderPass(vulkanRenderDevice->GetVkDevice(), &vk_renderpass_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &renderPass);
}

void VulkanRenderPass::Destroy(RenderDevice* renderDevice)
{
	auto vk_render_device = static_cast<VulkanRenderDevice*>(renderDevice);
	vkDestroyRenderPass(vk_render_device->GetVkDevice(), renderPass, vk_render_device->GetVkAllocationCallbacks());
}
