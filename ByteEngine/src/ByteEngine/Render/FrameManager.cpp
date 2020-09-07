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

			renderPassData.AttachmentNames.EmplaceBack(e.Name);
		}
		
		renderPassCreateInfo.RenderPassAttachments = attachmentDescriptors;
	}

	GTSL::Array<RenderPass::SubPassDescriptor, 8> subPassDescriptors;
	GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> readAttachmentReferences(subPassData.ElementCount());
	GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> writeAttachmentReferences(subPassData.ElementCount());
	GTSL::Array<GTSL::Array<uint8, 8>, 8> preserveAttachmentReferences(subPassData.ElementCount());

	subPasseses.EmplaceBack();
	subPassMap.EmplaceBack();
	
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

		{
			auto isUsed = [&](Id name) -> bool
			{
				bool result = false;
				
				for(uint8 i = s + static_cast<uint8>(1); i < subPassData.ElementCount(); ++i)
				{
					for(auto e : subPassData[s].ReadAttachments)
					{
						if (e == name) { result = true; }
					}

					for(auto e : subPassData[s].WriteAttachments)
					{
						if (e == name) { result = true; }
					}
				}

				return result;
			};

			for(uint32 a = 0; a < attachmentInfos.ElementCount(); ++a)
			{
				if(isUsed(attachmentInfos[a].Name))
				{
					preserveAttachmentReferences[s].EmplaceBack(a);
				}
			}
		}
		
		subPassDescriptor.PreserveAttachments = preserveAttachmentReferences[s];
		
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
		
		subPasseses.back().EmplaceBack();
		subPassMap.back().Emplace(subPassData[s].Name, s);
	}

	renderPassCreateInfo.SubPasses = subPassDescriptors;

	GTSL::Array<RenderPass::SubPassDependency, 8> subPassDependencies(subPassData.ElementCount());
	{
		uint8 subPass = 0;
		
		{
			auto& e = subPassDependencies[0];
			e.SourceSubPass = RenderPass::EXTERNAL;
			e.DestinationSubPass = subPass;
			
			e.SourceAccessFlags = 0;
			e.DestinationAccessFlags = AccessFlags::INPUT_ATTACHMENT_READ | AccessFlags::COLOR_ATTACHMENT_READ | AccessFlags::COLOR_ATTACHMENT_WRITE | AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ | AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE;

			e.SourcePipelineStage = PipelineStage::TOP_OF_PIPE;
			e.DestinationPipelineStage = PipelineStage::ALL_GRAPHICS;

			++subPass;
		}

		for (auto* begin = subPassDependencies.begin() + subPass; begin != subPassDependencies.end() - 1; ++begin)
		{
			auto& e = *begin;
			e.SourceSubPass = subPass;
			e.DestinationSubPass = subPass + 1;

			e.SourceAccessFlags = AccessFlags::INPUT_ATTACHMENT_READ | AccessFlags::COLOR_ATTACHMENT_READ | AccessFlags::COLOR_ATTACHMENT_WRITE | AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ | AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE;
			e.DestinationAccessFlags = 0;

			e.SourcePipelineStage = PipelineStage::ALL_GRAPHICS;
			e.DestinationPipelineStage = PipelineStage::BOTTOM_OF_PIPE;

			++subPass;
		}

		if(subPass < subPassDependencies.GetLength())
		{
			auto& e = subPassDependencies[subPass];
			
			e.SourceSubPass = subPass;
			e.DestinationSubPass = RenderPass::EXTERNAL;
			
			e.SourceAccessFlags = AccessFlags::INPUT_ATTACHMENT_READ | AccessFlags::COLOR_ATTACHMENT_READ | AccessFlags::COLOR_ATTACHMENT_WRITE | AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ | AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE;
			e.DestinationAccessFlags = 0;

			e.SourcePipelineStage = PipelineStage::ALL_GRAPHICS;
			e.DestinationPipelineStage = PipelineStage::BOTTOM_OF_PIPE;
		}
	}
	
	renderPassCreateInfo.SubPassDependencies = subPassDependencies;

	renderPassData.RenderPass = RenderPass(renderPassCreateInfo);
}

void FrameManager::OnResize(TaskInfo taskInfo, const GTSL::Extent2D newSize)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	auto resize = [&](Attachment& attachment) -> void
	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { textureCreateInfo.Name = attachment.Name.GetString(); }
		textureCreateInfo.Extent = { newSize.Width, newSize.Height, 1 };
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
	
	for(auto& renderPass : renderPasses)
	{
		FrameBuffer::CreateInfo framebufferCreateInfo;
		framebufferCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { framebufferCreateInfo.Name = "FrameBuffer"; }

		GTSL::Array<TextureView, 8> textureViews;

		for(auto e : renderPass.AttachmentNames) { textureViews.EmplaceBack(attachments.At(e).TextureView); }
		
		framebufferCreateInfo.TextureViews = textureViews;
		framebufferCreateInfo.RenderPass = &renderPass.RenderPass;
		framebufferCreateInfo.Extent = renderSystem->GetRenderExtent();

		renderPass.FrameBuffer = FrameBuffer(framebufferCreateInfo);
	}
}