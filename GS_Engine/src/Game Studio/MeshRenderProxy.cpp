#include "MeshRenderProxy.h"

MeshRenderProxy::MeshRenderProxy(VBO * VertexBuffer, IBO * IndexBuffer, VAO * VertexArray) :
	VertexBuffer(VertexBuffer),
	IndexBuffer(IndexBuffer),
	VertexArray(VertexArray)
{
}

MeshRenderProxy::MeshRenderProxy(WorldObject * Owner, VBO * VertexBuffer, IBO * IndexBuffer, VAO * VertexArray) :
	RenderProxy(Owner), VertexBuffer(VertexBuffer),
	IndexBuffer(IndexBuffer),
	VertexArray(VertexArray)
{
}