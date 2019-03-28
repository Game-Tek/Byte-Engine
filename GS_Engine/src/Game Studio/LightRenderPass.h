#pragma once

#include "Core.h"

#include "RenderPass.h"

#include "Program.h"

GS_CLASS LightRenderPass : public RenderPass
{
public:
	LightRenderPass(Renderer * RendererOwner);
	~LightRenderPass();

protected:
	Program LightingPassProgram;
};

