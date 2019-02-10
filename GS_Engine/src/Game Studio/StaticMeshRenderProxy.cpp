#include "StaticMeshRenderProxy.h"

#include "StaticMesh.h"

//TODO: PUT CODE IN INITIALIZER LIST.

StaticMeshRenderProxy::StaticMeshRenderProxy(WorldObject * Owner) : RenderProxy(Owner)
{
	//Create new Vertex buffer object to store this static mesh data.
	//Cast Owner to StaticMesh pointer, GetMeshResource pointer, Get Pointer to the data; Cast Owner to StaticMesh pointer, GetMeshResource pointer, Get the size of the data, Set render mode.
	VertexBuffer = new VBO(dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetData(), (uint32)dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetDataSize(), GL_STATIC_DRAW);

	//Create new IndexBuffer object to store this static mesh indeces.
	//					Cast Owner to Static	Get S.M. resource	Get Mesh	Get a pointer
	//					Mesh pointer.			pointer.			Data.		to the Index Array.
	IndexBuffer = new IBO(dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetMeshData()->IndexArray, dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetMeshData()->IndexCount);
}

StaticMeshRenderProxy::~StaticMeshRenderProxy()
{
	//Delete the buffers on the destruction of this object.
	delete VertexBuffer;
	delete IndexBuffer;
}