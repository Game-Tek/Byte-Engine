#pragma once

#include "Core.h"

#include "Buffer.h"

#include "glad.h"

GS_CLASS VAO : Buffer
{
public:
	VAO();
	~VAO();

	void Bind() override;
	void Enable() override;
	void CreateVertexAttributes();
private:
	unsigned short VertexAttributeIndex;
};

