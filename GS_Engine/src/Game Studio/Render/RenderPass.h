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

//Describes a subpass.
GS_STRUCT SubPassDescriptor
{
	AttachmentReference ReadColorAttachments[8];
	uint8 ColorAttachmentsCount = 0;
	uint32 PreserveAttachments[8];
	uint8 PreserveAttachmentsCount = 0;
};

//Describes a render pass.
GS_STRUCT RenderPassDescriptor
{
	AttachmentDescriptor ColorAttachments[8];
	uint8 ColorAttachmentsCount = 0;
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
	RenderPass() = default;
	~RenderPass() = default;
};