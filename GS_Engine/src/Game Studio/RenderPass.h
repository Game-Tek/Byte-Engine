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

	uint16 GetDrawCalls() const { return DrawCalls; }

protected:
	Renderer * RendererOwner;

	uint16 DrawCalls;
};