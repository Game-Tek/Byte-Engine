#pragma once

#include "Core.h"

#include "RendererObject.h"

class Texture;

GS_CLASS FrameBuffer : public RendererObject
{
public:
	FrameBuffer();
	~FrameBuffer();

	void Bind() const override;
	void UnBind() const override;

	void AttachTexture(const Texture & Texture);
	void AttachTexture(Texture * Texture);

private:
	//Keeps track of how many textures have been bound.
	uint8 BoundTextures = 0;
};

