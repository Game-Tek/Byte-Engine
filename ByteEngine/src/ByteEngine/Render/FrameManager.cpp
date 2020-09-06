#include "FrameManager.h"

#include "RenderSystem.h"

void FrameManager::AddAttachment(RenderSystem* renderSystem, const Id name, TextureFormat format, TextureUses::value_type uses, TextureType::value_type type)
{
	Attachment attachment;
	attachment.Format = format;
	attachment.Name = name;
	attachment.Type = type;
	attachment.Uses = uses;

	if(type & TextureType::DEPTH)
	{
		attachment.ClearValue = GTSL::RGBA(1, 0, 1, 1);
	}
	else
	{
		attachment.ClearValue = GTSL::RGBA(0, 0, 0, 0);
	}
	
	attachments.Emplace(name, attachment);
}

void FrameManager::AddPass(RenderSystem* renderSystem, const Id name, const GTSL::Ranger<const AttachmentInfo> attachmentInfos, const GTSL::Ranger<const SubPassData> subPassData)
{
	renderPassesMap.Emplace(name, renderPasses.GetLength());
	auto& renderPassData = renderPasses[renderPasses.EmplaceBack()];
	
	RenderPass::CreateInfo renderPassCreateInfo;
	renderPassCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
	if constexpr (_DEBUG) { renderPassCreateInfo.Name = "RenderPass"; }

	FrameBuffer::CreateInfo framebufferCreateInfo;
	framebufferCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
	if constexpr (_DEBUG) { framebufferCreateInfo.Name = "FrameBuffer"; }

	GTSL::Array<TextureView, 16> textureViews;
	
	{
		GTSL::Array<RenderPass::AttachmentDescriptor, 16> attachmentDescriptors;
		
		for(auto e : attachmentInfos)
		{
			RenderPassAttachment renderPassAttachment;
			renderPassAttachment.Index = attachmentDescriptors.GetLength();
			renderPassData.Attachments.Emplace(e.Name, renderPassAttachment);

			auto& attachment = attachments.At(e.Name);
			
			RenderPass::AttachmentDescriptor attachmentDescriptor;
			attachmentDescriptor.Format = attachment.Format;
			attachmentDescriptor.LoadOperation = e.Load;
			attachmentDescriptor.StoreOperation = e.Store;
			attachmentDescriptor.InitialLayout = e.StartState;
			attachmentDescriptor.FinalLayout = e.EndState;
			attachmentDescriptors.EmplaceBack(attachmentDescriptor);

			renderPassData.ClearValues.EmplaceBack(attachment.ClearValue);

			textureViews.EmplaceBack(attachment.TextureView);
		}
		
		renderPassCreateInfo.Descriptor.RenderPassAttachments = attachmentDescriptors;
	}

	GTSL::Array<RenderPass::SubPassDescriptor, 8> subPassDescriptors(subPassData.ElementCount());
	GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> readAttachmentReferences(subPassData.ElementCount());
	GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> writeAttachmentReferences(subPassData.ElementCount());

	for (uint32 s = 0; s < subPassData.ElementCount(); ++s)
	{
		RenderPass::SubPassDescriptor subPassDescriptor;

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

		subPassDescriptor.ReadColorAttachments = readAttachmentReferences[s];
		
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
			
		subPassDescriptor.WriteColorAttachments = writeAttachmentReferences[s];

		if (subPassData[s].DepthStencilAttachment.Name)
		{
			auto& attachmentInfo = renderPassData.Attachments.At(subPassData[s].DepthStencilAttachment.Name);
			
			subPassDescriptor.DepthAttachmentReference.Index = attachmentInfo.Index;
			subPassDescriptor.DepthAttachmentReference.Layout = subPassData[s].DepthStencilAttachment.Layout;
		}
		else
		{
			subPassDescriptor.DepthAttachmentReference.Index = GAL::ATTACHMENT_UNUSED;
			subPassDescriptor.DepthAttachmentReference.Layout = TextureLayout::UNDEFINED;
		}
			
		subPassDescriptors.EmplaceBack(subPassDescriptor);
	}

	renderPassCreateInfo.Descriptor.SubPasses = subPassDescriptors;

	renderPassData.RenderPass = RenderPass(renderPassCreateInfo);

	framebufferCreateInfo.TextureViews = textureViews;
	framebufferCreateInfo.RenderPass = &renderPassData.RenderPass;
	framebufferCreateInfo.Extent = renderSystem->GetRenderExtent();

	renderPassData.FrameBuffer = FrameBuffer(framebufferCreateInfo);
}

void FrameManager::OnResize(TaskInfo taskInfo, const GTSL::Extent2D newSize)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	auto resize = [&](Attachment& attachment) -> void
	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { textureCreateInfo.Name = attachment.Name.GetString(); }
		textureCreateInfo.Extent = renderSystem->GetRenderExtent();
		textureCreateInfo.Dimensions = Dimensions::SQUARE;
		textureCreateInfo.Format = attachment.Format;
		textureCreateInfo.MipLevels = 1;
		textureCreateInfo.Uses = attachment.Uses;
		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
		attachment.Texture = Texture(textureCreateInfo);

		RenderSystem::AllocateLocalTextureMemoryInfo allocateLocalTextureMemoryInfo;
		allocateLocalTextureMemoryInfo.Texture = attachment.Texture;
		allocateLocalTextureMemoryInfo.Allocation = &attachment.Allocation;
		renderSystem->AllocateLocalTextureMemory(allocateLocalTextureMemoryInfo);

		TextureView::CreateInfo textureViewCreateInfo;
		textureViewCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { textureViewCreateInfo.Name = attachment.Name.GetString(); }
		textureViewCreateInfo.Dimensions = Dimensions::SQUARE;
		textureViewCreateInfo.Format = attachment.Format;
		textureViewCreateInfo.MipLevels = 1;
		textureViewCreateInfo.Type = attachment.Type;
		textureViewCreateInfo.Texture = attachment.Texture;
		attachment.TextureView = TextureView(textureViewCreateInfo);

		TextureSampler::CreateInfo textureSamplerCreateInfo;
		textureSamplerCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		textureSamplerCreateInfo.Anisotropy = 0;
		if constexpr (_DEBUG) { textureSamplerCreateInfo.Name = attachment.Name.GetString(); }
		attachment.TextureSampler = TextureSampler(textureSamplerCreateInfo);
	};

	renderSystem->Wait();
	
	GTSL::ForEach(attachments, resize);
}