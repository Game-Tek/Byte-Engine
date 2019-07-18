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

	ImageLayout Layout = ImageLayout::GENERAL;

	Format AttachmentFormat;
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
GS_STRUCT SubPassDescriptor : PassDescriptor
{
	AttachmentReference ReadColorAttachments[8];
	uint32 PreserveAttachments[8];
	uint8 PreserveAttachmentsCount = 0;
};

//Describes a render pass.
GS_STRUCT RenderPassDescriptor : PassDescriptor
{
	AttachmentDescriptor ColorAttachments[8];
	AttachmentDescriptor DepthStencilAttachment;

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
	RenderPass();
	virtual ~RenderPass();
};