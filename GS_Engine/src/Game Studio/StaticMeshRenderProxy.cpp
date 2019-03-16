#include "StaticMeshRenderProxy.h"

#include "StaticMesh.h"

#include "StaticMeshResource.h"

#include "VBO.h"
#include "IBO.h"
#include "VAO.h"
#include <GLAD/glad.h>
#include "GL.h"

//TODO: CHECK HOW GETDATASIZE() WORKS.

StaticMeshRenderProxy::StaticMeshRenderProxy(WorldObject * Owner, const void * MeshData, size_t DataSize, const void * IndexData, uint32 IndexCount) : MeshRenderProxy(Owner, new VBO(MeshData, DataSize), new IBO(IndexData, IndexCount), new VAO(sizeof(Vertex)))
{
	VertexArray->Bind();
	VertexArray->CreateVertexAttribute(3, GL_FLOAT, false, sizeof(Vector3));
	VertexArray->CreateVertexAttribute(3, GL_FLOAT, false, sizeof(Vector3));
	VertexArray->CreateVertexAttribute(2, GL_FLOAT, false, sizeof(float) * 2);
	VertexArray->CreateVertexAttribute(3, GL_FLOAT, false, sizeof(Vector3));
	VertexArray->CreateVertexAttribute(3, GL_FLOAT, false, sizeof(Vector3));
}

void StaticMeshRenderProxy::Draw()
{
	IndexBuffer->Bind();
	VertexArray->Bind();

	GS_GL_CALL(glDrawElements(GL_TRIANGLES, IndexBuffer->GetCount(), GL_UNSIGNED_INT, nullptr));
}
