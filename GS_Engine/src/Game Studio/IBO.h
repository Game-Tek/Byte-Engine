#pragma once

#include "Core.h"

#include "RendererObject.h"

#define GL_STATIC_DRAW 0x88E4

GS_CLASS IBO : RendererObject

{
public:
	//Generates a new buffer.
	IBO();
	~IBO();

	//Makes this buffer the currently bound buffer.
	void Bind(int Usage = GL_STATIC_DRAW) const;
private:
	unsigned int IndexCount;
};

