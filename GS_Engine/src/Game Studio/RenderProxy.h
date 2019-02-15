#pragma once

#include "Core.h"

#include "VBO.h"
#include "IBO.h"

GS_CLASS RenderProxy
{
public:
	RenderProxy(VBO * VertexBuffer, IBO * IndexBuffer);

	VBO * GetVertexBuffer() { return VertexBuffer; }
	IBO * GetIndexBuffer() { return IndexBuffer; }

protected:
	VBO * VertexBuffer = nullptr;
	IBO * IndexBuffer = nullptr;
};