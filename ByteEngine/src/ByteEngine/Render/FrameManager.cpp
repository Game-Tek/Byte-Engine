#include "FrameManager.h"

#include "RenderSystem.h"

void FrameManager::AddAttachment(RenderSystem* renderSystem, const Id name, TextureFormat format, TextureUses uses, TextureType type)
{
	Attachment attachment;
	attachment.Format = format;

	Texture::CreateInfo depthTextureCreateInfo;
	depthTextureCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
	if constexpr (_DEBUG) { depthTextureCreateInfo.Name = name.GetString(); }
	depthTextureCreateInfo.Extent = renderSystem->GetRenderExtent();
	depthTextureCreateInfo.Dimensions = Dimensions::SQUARE;
	depthTextureCreateInfo.Format = format;
	depthTextureCreateInfo.MipLevels = 1;
	depthTextureCreateInfo.Uses = uses;
	depthTextureCreateInfo.Tiling = TextureTiling::OPTIMAL;
	depthTextureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
	attachment.Texture = Texture(depthTextureCreateInfo);

	RenderSystem::AllocateLocalTextureMemoryInfo allocateLocalTextureMemoryInfo;
	allocateLocalTextureMemoryInfo.Texture = attachment.Texture;
	allocateLocalTextureMemoryInfo.Allocation = &attachment.Allocation;
	renderSystem->AllocateLocalTextureMemory(allocateLocalTextureMemoryInfo);

	TextureView::CreateInfo depthTextureViewCreateInfo;
	depthTextureViewCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
	if constexpr (_DEBUG) { depthTextureViewCreateInfo.Name = name.GetString(); }
	depthTextureViewCreateInfo.Dimensions = Dimensions::SQUARE;
	depthTextureViewCreateInfo.Format = format;
	depthTextureViewCreateInfo.MipLevels = 1;
	depthTextureViewCreateInfo.Type = type;
	depthTextureViewCreateInfo.Texture = attachment.Texture;
	attachment.TextureView = TextureView(depthTextureViewCreateInfo);
	
	attachments.Emplace(name, attachment);
}

void FrameManager::AddPass(RenderSystem* renderSystem, const Id name, const GTSL::Ranger<const AttachmentInfo> read, const GTSL::Ranger<const AttachmentInfo> writes, const GTSL::Ranger<const SubPassData> subPassData)
{
	auto& renderPassData = renderPasses[renderPasses.EmplaceBack()];
	
	RenderPass::CreateInfo renderPassCreateInfo;
	renderPassCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
	if constexpr (_DEBUG) { renderPassCreateInfo.Name = "RenderPass"; }

	{
		GTSL::Array<RenderPass::AttachmentDescriptor, 16> attachmentDescriptors;
		
		for(auto e : read)
		{
			RenderPassAttachment renderPassAttachment;
			renderPassAttachment.Index = attachmentDescriptors.GetLength();
			renderPassData.Attachments.Emplace(e.Name, renderPassAttachment);
			
			RenderPass::AttachmentDescriptor attachmentDescriptor;
			attachmentDescriptor.Format = attachments.At(e.Name).Format;
			attachmentDescriptor.LoadOperation = e.Load;
			attachmentDescriptor.StoreOperation = e.Store;
			attachmentDescriptor.InitialLayout = e.StartState;
			attachmentDescriptor.FinalLayout = e.EndState;
			attachmentDescriptors.EmplaceBack(attachmentDescriptor);
		}

		for(auto e : writes)
		{
			RenderPassAttachment renderPassAttachment;
			renderPassAttachment.Index = attachmentDescriptors.GetLength();
			renderPassData.Attachments.Emplace(e.Name, renderPassAttachment);
			
			RenderPass::AttachmentDescriptor attachmentDescriptor;
			attachmentDescriptor.Format = attachments.At(e.Name).Format;
			attachmentDescriptor.LoadOperation = e.Load;
			attachmentDescriptor.StoreOperation = e.Store;
			attachmentDescriptor.InitialLayout = e.StartState;
			attachmentDescriptor.FinalLayout = e.EndState;
			attachmentDescriptors.EmplaceBack(attachmentDescriptor);
		}
		
		renderPassCreateInfo.Descriptor.RenderPassColorAttachments = attachmentDescriptors;
	}
	
	renderPassCreateInfo.Descriptor.DepthStencilAttachment = RenderPass::AttachmentDescriptor{ TextureFormat::DEPTH24_STENCIL8, GAL::RenderTargetLoadOperations::CLEAR, GAL::RenderTargetStoreOperations::UNDEFINED, TextureLayout::UNDEFINED, TextureLayout::DEPTH_STENCIL_ATTACHMENT };

	GTSL::Array<RenderPass::SubPassDescriptor, 8> subPassDescriptors;
	GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> readAttachmentReferences;
	GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> writeAttachmentReferences;

	for (uint32 s = 0; s < subPassData.ElementCount(); ++s)
	{
		RenderPass::SubPassDescriptor subPassDescriptor;

		{	
			for (auto e : subPassData[s].ReadAttachments)
			{
				auto& renderpassAttachment = renderPassData.Attachments.At(e);

				renderpassAttachment.Layout = subPassData[s].ReadAttachmentsLayouts[readAttachmentReferences[s].GetLength()];
				renderpassAttachment.Index = renderPassData.Attachments.At(e).Index;

				RenderPass::AttachmentReference attachmentReference;
				attachmentReference.Layout = renderpassAttachment.Layout;
				attachmentReference.Index = renderpassAttachment.Index;

				readAttachmentReferences[s].EmplaceBack(attachmentReference);
			}
		}

		{			
			for (auto e : subPassData[s].WriteAttachments)
			{
				auto& renderpassAttachment = renderPassData.Attachments.At(e);

				renderpassAttachment.Layout = subPassData[s].WriteAttachmentsLayouts[writeAttachmentReferences[s].GetLength()];
				renderpassAttachment.Index = renderPassData.Attachments.At(e).Index;

				RenderPass::AttachmentReference attachmentReference;
				attachmentReference.Layout = renderpassAttachment.Layout;
				attachmentReference.Index = renderpassAttachment.Index;

				writeAttachmentReferences[s].EmplaceBack(attachmentReference);
			}
		}
			
		subPassDescriptor.ReadColorAttachments = readAttachmentReferences[s];
		subPassDescriptor.WriteColorAttachments = writeAttachmentReferences[s];

		subPassDescriptors.EmplaceBack(subPassDescriptor);
	}

	renderPassCreateInfo.Descriptor.SubPasses = subPassDescriptors;

	renderPassData.RenderPass = RenderPass(renderPassCreateInfo);
}
