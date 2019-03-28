#pragma once

#include "Core.h"

#include "RenderPass.h"

#include "Program.h"
#include "Uniform.h"

#include "FrameBuffer.h"
#include "Texture.h"

GS_CLASS GBufferRenderPass : public RenderPass
{
public:
	GBufferRenderPass(Renderer * RendererOwner);
	~GBufferRenderPass();

	void Render() override;

protected:
	void SetAsActive() const override;

private:
	Program GBufferPassProgram;

	Uniform ViewMatrix;
	Uniform ProjMatrix;

	FrameBuffer GBuffer;
	
	Texture Position;
	Texture Normal;
	Texture Albedo;
};

