#pragma once

#include "Core.h"

#include "RenderProxy.h"

#include "VBO.h"
#include "IBO.h"
#include "VAO.h"

GS_CLASS MeshRenderProxy : public RenderProxy
{
public:
	MeshRenderProxy(VBO * VertexBuffer, IBO * IndexBuffer, VAO * VertexArray);
	MeshRenderProxy(WorldObject * Owner, VBO * VertexBuffer, IBO * IndexBuffer, VAO * VertexArray);
	virtual ~MeshRenderProxy();

	VBO * GetVertexBuffer() const { return VertexBuffer; }
	IBO * GetIndexBuffer() const { return IndexBuffer; }
	VAO * GetVertexArray() const { return VertexArray; }

protected:
	VBO * VertexBuffer = nullptr;
	IBO * IndexBuffer = nullptr;
	VAO * VertexArray = nullptr;
};

