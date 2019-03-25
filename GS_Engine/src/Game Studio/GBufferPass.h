#pragma once

#include "Core.h"

#include "RenderPass.h"

#include "FrameBuffer.h"
#include "Texture.h"

GS_CLASS GBufferPass : public RenderPass
{
public:
	GBufferPass();
	~GBufferPass();

	void SetAsActive() const override;

private:
	FrameBuffer GBuffer;
	
	Texture Position;
	Texture Normal;
	Texture Albedo;
};

