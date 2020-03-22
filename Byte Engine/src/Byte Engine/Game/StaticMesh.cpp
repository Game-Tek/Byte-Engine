#include "StaticMesh.h"

#include "Resources/StaticMeshResourceManager.h"
#include "Application/Application.h"

StaticMesh::StaticMesh(const FString& _Name)
{
	staticMeshResource = BE::Application::Get()->GetResourceManager()->TryGetResource(_Name, "Static Mesh");
}

StaticMesh::~StaticMesh()
{
	BE::Application::Get()->GetResourceManager()->ReleaseResource(staticMeshResource);
}

//Model StaticMesh::GetModel() const
//{
//	return Model{ staticMeshResource->GetStaticMeshData().VertexArray, staticMeshResource->GetStaticMeshData().IndexArray, staticMeshResource->GetStaticMeshData().VertexCount, staticMeshResource->GetStaticMeshData().IndexCount };
//}
