#pragma once

#include "Core.h"

#include "RenderCore.h"

//Base class that describes an attachment.
GS_STRUCT AttachmentDescriptor
{
	AttachmentDescriptor(LoadOperations _LOp, StoreOperations _SOp) : LoadOperation(_LOp), StoreOperation(_SOp)
	{
	}
	
	//Defines the operation that should be run when the attachment is loaded for rendering.
	LoadOperations LoadOperation = LoadOperations::UNDEFINED;
	//Defines the operation that should be run when the attachment is done being rendered to.
	StoreOperations StoreOperation = StoreOperations::STORE;
};

//Describes a color attachment.
GS_STRUCT ColorAttachmentDescriptor : public AttachmentDescriptor
{
	ColorAttachmentDescriptor() = default;
	ColorAttachmentDescriptor(LoadOperations _LOp, StoreOperations _SOp, ColorFormat _CF) : AttachmentDescriptor(_LOp, _SOp), Format(_CF)
	{
	}

	//Defines the format of this color attachment.
	ColorFormat Format = ColorFormat::BGRA_I8;
};

//Describes a depth attachment.
GS_STRUCT DepthAttachmentDescriptor : public AttachmentDescriptor
{
	DepthAttachmentDescriptor() = default;
	DepthAttachmentDescriptor(LoadOperations _LOp, StoreOperations _SOp, DepthStencilFormat _DSF) : AttachmentDescriptor(_LOp, _SOp), Format(_DSF)
	{
	}

	//Defines the format of this depth stencil attachment.
	DepthStencilFormat Format = DepthStencilFormat::DEPTH16_STENCIL8;
};

//Describes the reference to a render pass attachment for a sub pass.
GS_STRUCT AttachmentReference
{
	uint8 Index = 0;
	ImageLayout Layout = ImageLayout::COLOR_ATTACHMENT;
};

//Base class that describes a pass (render pass).
GS_STRUCT PassDescriptor
{
	uint8 ColorAttachmentsCount = 0;
};

//Describes a subpass.
GS_STRUCT SubPassDescriptor : public PassDescriptor
{
	AttachmentReference ReadColorAttachments[8];
	uint32 PreserveAttachments[8];
	uint8 PreserveAttachmentsCount = 0;
};

//Describes a render pass.
GS_STRUCT RenderPassDescriptor : public PassDescriptor
{
	ColorAttachmentDescriptor ColorAttachments[8];
	DepthAttachmentDescriptor DepthStencilAttachment;

	SubPassDescriptor SubPasses[8];
	uint8 SubPassesCount = 1;
};

GS_STRUCT RenderPassCreateInfo
{
	RenderPassDescriptor RPDescriptor;
};

GS_CLASS RenderPass
{
public:
	RenderPass(const RenderPassDescriptor& _RPD);
	virtual ~RenderPass();
};