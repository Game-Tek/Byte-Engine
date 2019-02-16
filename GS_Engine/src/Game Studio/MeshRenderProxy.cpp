#include "MeshRenderProxy.h"

MeshRenderProxy::MeshRenderProxy(WorldObject * Owner, VBO * VertexBuffer, IBO * IndexBuffer, VAO * VertexArray) :
	RenderProxy(Owner), VertexBuffer(VertexBuffer),
	IndexBuffer(IndexBuffer),
	VertexArray(VertexArray)
{
}


MeshRenderProxy::~MeshRenderProxy()
{
	delete VertexBuffer;
	delete IndexBuffer;
	delete VertexArray;
}
