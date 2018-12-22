#pragma once

#include "Core.h"

#include "Buffer.h"

#include "glad.h"

GS_CLASS VAO : Buffer
{
public:
	VAO();
	~VAO();

	void Bind();
	void CreateVertexAttribute();
private:

};

