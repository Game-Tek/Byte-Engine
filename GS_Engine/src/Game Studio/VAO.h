#pragma once

#include "Core.h"

#include "RendererObject.h"

GS_CLASS VAO : public RendererObject
{
public:
	VAO();
	~VAO();

	void Bind() const override;

	template <typename VertexType>
	void CreateVertexAttribute(int NumberOfElementsInThisAttribute, int DataType, int Normalize, void * Offset);
private:
	unsigned short VertexAttributeIndex = 0;
};

