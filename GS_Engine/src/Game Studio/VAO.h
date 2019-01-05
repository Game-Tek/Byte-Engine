#pragma once

#include "Core.h"

#include "RendererObject.h"

GS_CLASS VAO : RendererObject
{
public:
	VAO();
	~VAO();

	void Bind() const;
	void CreateVertexAttribute(int NumberOfElementsInThisAttribute, int DataType, int Normalize, int Stride, void * Offset);
private:
	unsigned short VertexAttributeIndex = 0;
};

