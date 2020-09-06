#pragma once
#include <GTSL/Array.hpp>
#include <GTSL/StaticMap.hpp>

#include "RenderTypes.h"
#include "ByteEngine/Id.h"
#include "ByteEngine/Game/System.h"

struct TaskInfo;
class RenderSystem;

class FrameManager : public System
{
public:
	FrameManager() : System("FrameManager") {}

	void Initialize(const InitializeInfo& initializeInfo) override {}
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void AddAttachment(RenderSystem* renderSystem, Id name, TextureFormat format, TextureUses::value_type uses, TextureType::value_type type);
	GTSL::Ranger<const GTSL::RGBA> GetClearValues(const uint8 rp)
	{
		auto& renderPass = renderPasses[rp];
		return renderPass.ClearValues;
	}

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

		struct AttachmentUse
		{
			Id Name;
			TextureLayout Layout;
		};
		AttachmentUse DepthStencilAttachment;
	};
	void AddPass(RenderSystem* renderDevice, Id name, GTSL::Ranger<const AttachmentInfo> attachmentInfos, GTSL::Ranger<const SubPassData> subPassData);

	void OnResize(TaskInfo taskInfo, const GTSL::Extent2D newSize);
	
	[[nodiscard]] RenderPass GetRenderPass(const uint8 rp) const { return renderPasses[rp].RenderPass; }
	[[nodiscard]] RenderPass GetRenderPass(const Id rp) const { return renderPasses[renderPassesMap.At(rp)].RenderPass; }
	[[nodiscard]] FrameBuffer GetFrameBuffer(const uint8 rp) const { return renderPasses[rp].FrameBuffer; }
	[[nodiscard]] uint8 GetRenderPassCount() const { return renderPasses.GetLength(); }
	[[nodiscard]] uint8 GetSubPassCount(const uint8 renderPass) const { return subPasseses[renderPass].GetLength(); }
	
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
		GTSL::Array<GTSL::RGBA, 8> ClearValues;

		FrameBuffer FrameBuffer;
	};
	GTSL::Array<RenderPassData, 16> renderPasses;

	GTSL::StaticMap<uint8, 16> renderPassesMap;
	
	struct SubPass
	{
		uint8 DepthAttachment;
	};
	GTSL::Array<GTSL::Array<SubPass, 16>, 16> subPasseses;

	struct Attachment
	{
		TextureFormat Format;
		Texture Texture;
		TextureView TextureView;
		TextureSampler TextureSampler;

		GTSL::RGBA ClearValue;
		
		RenderAllocation Allocation;

		Id Name;
		TextureType::value_type Type;
		TextureUses::value_type Uses;
	};
	GTSL::StaticMap<Attachment, 32> attachments;

public:
	Texture GetAttachmentTexture(const Id attachment) const { return attachments.At(attachment).Texture; }
	TextureView GetAttachmentTextureView(const Id attachment) const { return attachments.At(attachment).TextureView; }
	TextureSampler GetAttachmentTextureSampler(const Id attachment) const { return attachments.At(attachment).TextureSampler; }
};
