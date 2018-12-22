#pragma once

#include "Core.h"

#include "Buffer.h"

#include "glad.h"

GS_CLASS VBO : Buffer
{
public:
	VBO();
	~VBO();

	void Bind(int Usage = GL_STATIC_DRAW);
};

