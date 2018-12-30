#pragma once

#include "Core.h"

#include "RendererObject.h"

GS_CLASS VAO : RendererObject
{
public:
	VAO();
	~VAO();

	void Bind() const override;
	void Enable() const override;
	void CreateVertexAttributes();
private:
	unsigned short VertexAttributeIndex;
};

