#include "MeshRenderProxy.h"

MeshRenderProxy::MeshRenderProxy(VBO* VertexBuffer, IBO* IndexBuffer, VAO* VertexArray) : VertexBuffer(VertexBuffer),
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
