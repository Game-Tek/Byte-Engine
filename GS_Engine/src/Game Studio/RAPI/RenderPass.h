#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Image.h"
#include "Containers/Array.hpp"

#define MAX_ATTACHMENTS_COUNT 8

//Describes the reference to a render pass attachment for a sub pass.
GS_STRUCT AttachmentReference
{
	//Id of the attachment (Index into RenderpassDescriptor::RenderPassColorAttachments).
	uint8 Index = 0;
	//Layout of the attachment during the sub pass.
	ImageLayout Layout = ImageLayout::COLOR_ATTACHMENT;
};

//Describes a subpass.
GS_STRUCT SubPassDescriptor
{
	//Array of AttachmentsReferences for attachments which the subpass reads from.
	Array<AttachmentReference, MAX_ATTACHMENTS_COUNT, uint8> ReadColorAttachments;

	//Array of AttachmentsReferences for attachments which the subpass writes to.
	Array<AttachmentReference, MAX_ATTACHMENTS_COUNT, uint8> WriteColorAttachments;

	//Array of indices identifying attachments that are not used by this subpass, but whose contents MUST be preserved throughout the subpass.
	Array<uint8, MAX_ATTACHMENTS_COUNT, uint8> PreserveAttachments;
};

//Describes a render pass.
GS_STRUCT RenderPassDescriptor
{
	//Array of pointer to images that will be used as attachments in the render pass.
	Array<Image*, MAX_ATTACHMENTS_COUNT, uint8> RenderPassColorAttachments;
	//Pointer to an image that will be used as the depth stencil attachment in the render pass.
	Image* DepthStencilAttachment = nullptr;

	//Array of SubpassDescriptor used to describes the properties of every subpass in the renderpass.
	Array<SubPassDescriptor, MAX_ATTACHMENTS_COUNT, uint8> SubPasses;
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