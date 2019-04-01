#pragma once

#include "Core.h"

#include "RenderPass.h"

#include "Program.h"

#include "Uniform.h"
#include "ScreenQuad.h"

GS_CLASS LightRenderPass : public RenderPass
{
public:
	LightRenderPass(Renderer * RendererOwner);
	~LightRenderPass();

	void SetAsActive() const override;
	void Render() override;

protected:
	Program LightingPassProgram;

	Uniform ViewMatrix;
	Uniform ProjMatrix;

	ScreenQuad Quad;
};

