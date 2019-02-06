#include "StaticMeshRenderProxy.h"

#include "StaticMesh.h"

StaticMeshRenderProxy::StaticMeshRenderProxy(WorldObject * Owner) : RenderProxy(Owner)
{
	//Create new Vertex buffer object to store this static mesh data.
	//Cast Owner to StaticMesh pointer, GetMeshResource pointer, Get Pointer to the data; Cast Owner to StaticMesh pointer, GetMeshResource pointer, Get the size of the data, Set render mode.
	VertexBuffer = new VBO(((StaticMesh *)Owner)->GetMeshResource()->GetData(), ((StaticMesh *)Owner)->GetMeshResource()->GetDataSize(), GL_STATIC_DRAW);

	//Create new IndexBuffer object to store this static mesh indeces.
	IndexBuffer = new IBO(((StaticMesh *)Owner)->GetMeshResource()->GetMeshData()->IndexArray, ((StaticMesh *)Owner)->GetMeshResource()->GetMeshData()->IndexCount);
}

StaticMeshRenderProxy::~StaticMeshRenderProxy()
{
	delete VertexBuffer;
	delete IndexBuffer;
}