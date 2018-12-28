#pragma once

#include "Core.h"

#include "Buffer.h"

#include "glad.h"

GS_CLASS EBO : Buffer

{
public:
	EBO();
	~EBO();

	void Bind(int Usage = GL_STATIC_DRAW);
};

