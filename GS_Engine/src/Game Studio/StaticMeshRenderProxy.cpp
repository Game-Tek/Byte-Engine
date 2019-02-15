#include "StaticMeshRenderProxy.h"

#include "StaticMesh.h"


StaticMeshRenderProxy::StaticMeshRenderProxy(StaticMeshResource * MeshResource) : RenderProxy( 
	new VBO(MeshResource->GetData(), MeshResource->GetDataSize(), GL_STATIC_DRAW),

	new IBO(MeshResource->GetMeshData()->IndexArray, MeshResource->GetMeshData()->IndexCount))
{
}

StaticMeshRenderProxy::~StaticMeshRenderProxy()
{
	//Delete the buffers on the destruction of this object.
	delete VertexBuffer;
	delete IndexBuffer;
}