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

	const Texture & GetPositionTexture() const { return Position; }
	const Texture & GetNormalTexture() const { return Normal; }
	const Texture & GetAlbedoTexture() const { return Albedo; }

protected:
	void SetAsActive() const override;

private:
	Program GBufferPassProgram;

	Uniform ViewMatrix;
	Uniform ProjMatrix;
	Uniform ModelMatrix;

	FrameBuffer GBuffer;
	
	Texture Position;
	Texture Normal;
	Texture Albedo;
};

