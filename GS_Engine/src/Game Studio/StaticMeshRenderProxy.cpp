#include "StaticMeshRenderProxy.h"

#include "StaticMesh.h"

#include "StaticMeshResource.h"

#include "VBO.h"
#include "IBO.h"
#include "VAO.h"
#include <GLAD/glad.h>
#include "GL.h"

//TODO: CHECK HOW GETDATASIZE() WORKS.

StaticMeshRenderProxy::StaticMeshRenderProxy(const void * MeshData, size_t DataSize, const void * IndexData, uint32 IndexCount) : MeshRenderProxy(new VBO(MeshData, DataSize), new IBO(IndexData, IndexCount), new VAO(sizeof(Vertex)))
{
	VertexArray->Bind();
	VertexArray->CreateVertexAttribute(3, GL_FLOAT, false, sizeof(Vector3));
}

StaticMeshRenderProxy::StaticMeshRenderProxy(WorldObject * Owner) : MeshRenderProxy(Owner, 
	new VBO(dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetMeshData(),
			dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetDataSize()),
	new IBO(dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetMeshData()->IndexArray,
			dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetMeshData()->IndexCount),
	new VAO(sizeof(Vertex)))
{
}

void StaticMeshRenderProxy::Draw()
{
	IndexBuffer->Bind();
	VertexArray->Bind();

	GS_GL_CALL(glDrawElements(GL_TRIANGLES, IndexBuffer->GetCount(), GL_UNSIGNED_INT, nullptr));
}
