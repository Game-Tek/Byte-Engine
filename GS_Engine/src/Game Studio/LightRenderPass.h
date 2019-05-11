#pragma once

#include "Core.h"

#include "RenderPass.h"

#include "LightingProgram.h"
#include "PointLightProgram.h"

#include "Uniform.h"
#include "ScreenQuad.h"

GS_CLASS LightRenderPass : public RenderPass
{
public:
	LightRenderPass(Renderer * RendererOwner);
	~LightRenderPass();

	void Render() override;

protected:
	LightingProgram LightingPassProgram;

	PointLightProgram PointLightProg;

	ScreenQuad Quad;
};

