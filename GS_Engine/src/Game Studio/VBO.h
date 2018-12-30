#pragma once

#include "Core.h"

#include "RendererObject.h"

#define GL_STATIC_DRAW 0x88E4

GS_CLASS VBO : RendererObject
{
public:
	VBO();
	~VBO();

	void Bind(int Usage = GL_STATIC_DRAW) const;
};

