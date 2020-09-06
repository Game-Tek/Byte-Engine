#pragma once
#include <GTSL/Array.hpp>
#include <GTSL/StaticMap.hpp>


#include "RenderTypes.h"
#include "ByteEngine/Id.h"

class RenderSystem;

class FrameManager
{
public:
	void AddAttachment(RenderSystem* renderSystem, const Id name, TextureFormat format, TextureUses uses, TextureType type);

	struct AttachmentInfo
	{
		Id Name;
		TextureLayout StartState, EndState;
		GAL::RenderTargetLoadOperations Load;
		GAL::RenderTargetStoreOperations Store;
	};
	
	struct SubPassData
	{
		GTSL::Array<Id, 8> ReadAttachments, WriteAttachments;
		GTSL::Array<TextureLayout, 8> ReadAttachmentsLayouts, WriteAttachmentsLayouts;
	};
	void AddPass(RenderSystem* renderDevice, Id name, GTSL::Ranger<const AttachmentInfo> read, GTSL::Ranger<const AttachmentInfo> writes, GTSL::Ranger<const SubPassData> subPassData);
	
private:
	struct RenderPassAttachment
	{
		TextureLayout Layout;
		uint8 Index;
	};
	
	struct RenderPassData
	{
		RenderPass RenderPass;
		GTSL::StaticMap<RenderPassAttachment, 8> Attachments;
	};
	GTSL::Array<RenderPassData, 16> renderPasses;
	struct SubPass
	{
		
	};
	GTSL::Array<GTSL::Array<SubPass, 16>, 16> subPasseses;

	struct Attachment
	{
		TextureFormat Format;
		Texture Texture;
		TextureView TextureView;
		TextureSampler TextureSampler;

		RenderAllocation Allocation;
	};
	GTSL::StaticMap<Attachment, 32> attachments;
};
