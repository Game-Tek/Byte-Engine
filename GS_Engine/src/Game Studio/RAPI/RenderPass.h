#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Image.h"
#include "Containers/Array.hpp"

#define ATTACHMENT_UNUSED 255

//Describes the reference to a render pass attachment for a sub pass.
struct GS_API AttachmentReference
{
	//Id of the attachment (Index into RenderpassDescriptor::RenderPassColorAttachments).
	uint8 Index = ATTACHMENT_UNUSED;
	//Layout of the attachment during the sub pass.
	ImageLayout Layout = ImageLayout::COLOR_ATTACHMENT;
};

struct GS_API AttachmentDescriptor
{
	//Pointer to the image this attachment refers to.
	Image* AttachmentImage = nullptr;
	//Defines the operation that should be run when the attachment is loaded for rendering.
	LoadOperations LoadOperation = LoadOperations::UNDEFINED;
	//Defines the operation that should be run when the attachment is done being rendered to.
	StoreOperations StoreOperation = StoreOperations::STORE;
	//Layout of the attachment when first used in the render pass.
	ImageLayout InitialLayout = ImageLayout::GENERAL;
	//Layout of the attachment after use in the render pass.
	ImageLayout FinalLayout = ImageLayout::GENERAL;
};

//Describes a subpass.
struct GS_API SubPassDescriptor
{
	//Array of AttachmentsReferences for attachments which the subpass reads from.
	Array<AttachmentReference*, 8> ReadColorAttachments;

	//Array of AttachmentsReferences for attachments which the subpass writes to.
	Array<AttachmentReference*, 8> WriteColorAttachments;

	//Array of indices identifying attachments that are not used by this subpass, but whose contents MUST be preserved throughout the subpass.
	Array<uint8, 8> PreserveAttachments;

	AttachmentReference* DepthAttachmentReference = nullptr;
};

//Describes a render pass.
struct GS_API RenderPassDescriptor
{
	//Array of pointer to images that will be used as attachments in the render pass.
	Array<AttachmentDescriptor*, 8> RenderPassColorAttachments;
	//Pointer to an image that will be used as the depth stencil attachment in the render pass.
	AttachmentDescriptor DepthStencilAttachment;

	//Array of SubpassDescriptor used to describes the properties of every subpass in the renderpass.
	Array<SubPassDescriptor*, 8> SubPasses;
};

struct GS_API RenderPassCreateInfo
{
	RenderPassDescriptor Descriptor;
};

class GS_API RenderPass
{
public:
	RenderPass() = default;
	~RenderPass() = default;
};
