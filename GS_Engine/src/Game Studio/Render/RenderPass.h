#pragma once

#include "Core.h"

//Describes all possible operations a renderer can perform when loading a render target onto a render pass.
enum class LoadOperations : uint8
{
	//We don't care about the previous content of the render target. Behavior is unknown.
	UNDEFINED,
	//We want to load the previous content of the render target.
	LOAD,
	//We want the render target to be cleared to black for color attachments and to 0 for depth/stencil attachments.
	CLEAR
};

//Describes all possible operations a renderer can perform when saving to a render target from a render pass.
enum class StoreOperations : uint8
{
	//We don't care about the outcome of the render target.
	UNDEFINED,
	//We want to store the result of the render pass to this render attachment.
	STORE
};

#include "RenderCore.h"

GS_STRUCT AttachmentDescriptor
{
	AttachmentDescriptor(LoadOperations _LOp, StoreOperations _SOp) : LoadOperation(_LOp), StoreOperation(_SOp)
	{
	}

	LoadOperations LoadOperation = LoadOperations::UNDEFINED;
	StoreOperations StoreOperation = StoreOperations::STORE;
};

GS_STRUCT ColorAttachmentDescriptor : public AttachmentDescriptor
{
	ColorAttachmentDescriptor() = default;
	ColorAttachmentDescriptor(LoadOperations _LOp, StoreOperations _SOp, ColorFormat _CF) : AttachmentDescriptor(_LOp, _SOp), Format(_CF)
	{
	}

	ColorFormat Format = ColorFormat::BGRA_I8;
};

GS_STRUCT DepthAttachmentDescriptor : public AttachmentDescriptor
{
	DepthAttachmentDescriptor() = default;
	DepthAttachmentDescriptor(LoadOperations _LOp, StoreOperations _SOp, DepthStencilFormat _DSF) : AttachmentDescriptor(_LOp, _SOp), Format(_DSF)
	{
	}

	DepthStencilFormat Format = DepthStencilFormat::DEPTH16_STENCIL8;
};

GS_STRUCT RenderPassDescriptor
{
	ColorAttachmentDescriptor ColorAttachments[8];
	DepthAttachmentDescriptor DepthStencilAttachment;
protected:

};

GS_CLASS RenderPass
{
public:
	virtual ~RenderPass();

	virtual void AddSubPass() = 0;
};