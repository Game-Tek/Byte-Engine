#include "StaticMeshRenderProxy.h"

#include "StaticMesh.h"

//TODO: CHECK HOW GETDATASIZE() WORKS.

StaticMeshRenderProxy::StaticMeshRenderProxy(WorldObject * Owner) : MeshRenderProxy(Owner, 
	new VBO(dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetMeshData(),
	        dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetDataSize()),
	new IBO(dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetMeshData()->IndexArray,
	        dynamic_cast<StaticMesh *>(Owner)->GetMeshResource()->GetMeshData()->IndexCount),
	new VAO(sizeof(Vertex)))
{
}

StaticMeshRenderProxy::~StaticMeshRenderProxy()
{

}