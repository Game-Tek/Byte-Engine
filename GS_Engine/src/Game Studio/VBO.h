#pragma once

#include "Core.h"

#include "RendererObject.h"

#define GL_STATIC_DRAW 0x88E4

GS_CLASS VBO : public RendererObject
{
public:
	VBO(const void * Data, unsigned int Size, int Usage = GL_STATIC_DRAW);
	~VBO();

	void Bind() const override;
};

