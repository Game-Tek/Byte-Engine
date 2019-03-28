#pragma once

#include "Core.h"

class Renderer;

GS_CLASS RenderPass
{
public:
	RenderPass(Renderer * RendererOwner);
	~RenderPass() = default;

	//
	//Call to actually perform rendering of all things comprised by this render pass.
	//
	//Should be later split into RenderStatic/RenderDynamic.
	//
	virtual void Render() = 0;

protected:
	//
	//Call to set all variables to prepare for rendering of this pass.
	//
	//Must be called before Render().
	//
	virtual void SetAsActive() const = 0;

	Renderer * RendererOwner;

	uint16 DrawCalls;
};

RenderPass::RenderPass(Renderer * RendererOwner) : RendererOwner(RendererOwner)
{
}