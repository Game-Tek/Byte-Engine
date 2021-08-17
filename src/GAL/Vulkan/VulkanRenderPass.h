#pragma once

#include "GAL/RenderPass.h"

#include "GAL/Vulkan/Vulkan.h"
#include "GAL/Vulkan/VulkanRenderDevice.h"
#include <GTSL/Range.h>

namespace GAL
{
	class VulkanRenderPass final : public RenderPass
	{
	public:
		VulkanRenderPass() = default;
		
		void Initialize(const VulkanRenderDevice* renderDevice, GTSL::Range<const RenderPassTargetDescription*> attachments,
			GTSL::Range<const SubPassDescriptor*> subPasses, const GTSL::Range<const SubPassDependency*> subPassDependencies) {
			GTSL::StaticVector<VkAttachmentDescription, 32> vkAttachmentDescriptions;

			for (GTSL::uint32 i = 0; i < static_cast<GTSL::uint32>(attachments.ElementCount()); ++i) {
				auto& attachmentDescription = vkAttachmentDescriptions.EmplaceBack();

				attachmentDescription.flags = 0;
				attachmentDescription.format = ToVulkan(MakeFormatFromFormatDescriptor(attachments[i].FormatDescriptor));
				attachmentDescription.samples = VK_SAMPLE_COUNT_1_BIT; //TODO: Should match that of the SwapChain images.
				attachmentDescription.loadOp = ToVkAttachmentLoadOp(attachments[i].LoadOperation);
				attachmentDescription.storeOp = ToVkAttachmentStoreOp(attachments[i].StoreOperation);
				attachmentDescription.stencilLoadOp = vkAttachmentDescriptions[i].loadOp;
				attachmentDescription.stencilStoreOp = vkAttachmentDescriptions[i].storeOp;
				attachmentDescription.initialLayout = ToVulkan(attachments[i].Start, attachments[i].FormatDescriptor);
				attachmentDescription.finalLayout = ToVulkan(attachments[i].End, attachments[i].FormatDescriptor);
			}

			GTSL::StaticVector<GTSL::StaticVector<VkAttachmentReference, 16>, 16> writeAttachmentsReferences;
			GTSL::StaticVector<GTSL::StaticVector<VkAttachmentReference, 16>, 16> readAttachmentsReferences;
			GTSL::StaticVector<GTSL::StaticVector<GTSL::uint32, 16>, 16> preserveAttachmentsIndices;
			GTSL::StaticVector<VkAttachmentReference, 16> vkDepthAttachmentReferences;

			for (GTSL::uint32 s = 0; s < static_cast<GTSL::uint32>(subPasses.ElementCount()); ++s) {
				writeAttachmentsReferences.EmplaceBack(); readAttachmentsReferences.EmplaceBack(); preserveAttachmentsIndices.EmplaceBack();

				auto& depthAttachment = vkDepthAttachmentReferences.EmplaceBack();
				depthAttachment.attachment = VK_ATTACHMENT_UNUSED; depthAttachment.layout = VK_IMAGE_LAYOUT_UNDEFINED;

				for (GTSL::uint32 a = 0; a < static_cast<GTSL::uint32>(subPasses[s].Attachments.ElementCount()); ++a) {
					VkAttachmentReference attachmentReference;
					attachmentReference.attachment = subPasses[s].Attachments[a].Index;
					attachmentReference.layout = ToVulkan(subPasses[s].Attachments[a].Layout, attachments[subPasses[s].Attachments[a].Index].FormatDescriptor);
					
					if (subPasses[s].Attachments[a].Access & AccessTypes::WRITE) {
						if (attachments[subPasses[s].Attachments[a].Index].FormatDescriptor.Type == TextureType::COLOR) {
							writeAttachmentsReferences[s].EmplaceBack(attachmentReference);
						} else {
							depthAttachment.attachment = attachmentReference.attachment;
							depthAttachment.layout = attachmentReference.layout;
						}
					} else {
						if (attachments[subPasses[s].Attachments[a].Index].FormatDescriptor.Type == TextureType::COLOR) {
							readAttachmentsReferences[s].EmplaceBack(attachmentReference);
						} else {
							depthAttachment.attachment = attachmentReference.attachment;
							depthAttachment.layout = attachmentReference.layout;
						}
					}
				}

				for (GTSL::uint32 a = 0; a < static_cast<GTSL::uint32>(subPasses[s].PreserveAttachments.ElementCount()); ++a) {
					preserveAttachmentsIndices[s].EmplaceBack(subPasses[s].PreserveAttachments[a]);
				}
			}

			GTSL::StaticVector<VkSubpassDescription, 32> vkSubpassDescriptions;

			for (GTSL::uint32 s = 0; s < static_cast<GTSL::uint32>(subPasses.ElementCount()); ++s) {
				auto& description = vkSubpassDescriptions.EmplaceBack();
				description.flags = 0;
				description.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
				description.colorAttachmentCount = writeAttachmentsReferences[s].GetLength();
				description.pColorAttachments = writeAttachmentsReferences[s].begin();
				description.inputAttachmentCount = readAttachmentsReferences[s].GetLength();
				description.pInputAttachments = readAttachmentsReferences[s].begin();
				description.pResolveAttachments = nullptr;
				description.preserveAttachmentCount = preserveAttachmentsIndices[s].GetLength();
				description.pPreserveAttachments = preserveAttachmentsIndices[s].begin();
				description.pDepthStencilAttachment = &vkDepthAttachmentReferences[s];
			}

			GTSL::StaticVector<VkSubpassDependency, 32> vkSubpassDependencies;
			for (GTSL::uint32 s = 0; s < static_cast<GTSL::uint32>(subPassDependencies.ElementCount()); ++s) {
				auto& dependency = vkSubpassDependencies.EmplaceBack();
				dependency.srcSubpass = subPassDependencies[s].SourceSubPass == EXTERNAL ? VK_SUBPASS_EXTERNAL : subPassDependencies[s].SourceSubPass;
				dependency.dstSubpass = subPassDependencies[s].DestinationSubPass == EXTERNAL ? VK_SUBPASS_EXTERNAL : subPassDependencies[s].DestinationSubPass;
				dependency.srcStageMask = ToVulkan(subPassDependencies[s].SourcePipelineStage);
				dependency.dstStageMask = ToVulkan(subPassDependencies[s].DestinationPipelineStage);
				dependency.dependencyFlags = 0;
			}

			VkRenderPassCreateInfo vkRenderpassCreateInfo{ VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO };
			vkRenderpassCreateInfo.attachmentCount = static_cast<GTSL::uint32>(attachments.ElementCount());
			vkRenderpassCreateInfo.pAttachments = vkAttachmentDescriptions.begin();
			vkRenderpassCreateInfo.subpassCount = static_cast<GTSL::uint32>(subPasses.ElementCount());
			vkRenderpassCreateInfo.pSubpasses = vkSubpassDescriptions.begin();
			vkRenderpassCreateInfo.dependencyCount = vkSubpassDependencies.GetLength();
			vkRenderpassCreateInfo.pDependencies = vkSubpassDependencies.begin();

			renderDevice->VkCreateRenderPass(renderDevice->GetVkDevice(), &vkRenderpassCreateInfo, renderDevice->GetVkAllocationCallbacks(), &renderPass);
			//setName(createInfo.RenderDevice, renderPass, VK_OBJECT_TYPE_RENDER_PASS, createInfo.Name);
		}
		

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyRenderPass(renderDevice->GetVkDevice(), renderPass, renderDevice->GetVkAllocationCallbacks());
			debugClear(renderPass);
		}
		
		~VulkanRenderPass() = default;

		[[nodiscard]] VkRenderPass GetVkRenderPass() const { return renderPass; }
		[[nodiscard]] uint64_t GetHandle() const { return reinterpret_cast<uint64_t>(renderPass); }
	private:
		VkRenderPass renderPass = nullptr;
	};
}
