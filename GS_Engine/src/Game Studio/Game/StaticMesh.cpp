#include "StaticMesh.h"

#include "Resources/StaticMeshResource.h"
#include "Application/Application.h"

StaticMesh::StaticMesh(const FString& _Name)
{
	GS::Application::Get()->GetResourceManager()->TryGetResource(_Name, "Static Mesh");
}

StaticMesh::~StaticMesh()
{
	GS::Application::Get()->GetResourceManager()->ReleaseResource("Static Mesh", "name");
}

Model StaticMesh::GetModel() const
{
	return Model{
		staticMeshResource->GetStaticMeshData().VertexArray, staticMeshResource->GetStaticMeshData().IndexArray,
		staticMeshResource->GetStaticMeshData().VertexCount, staticMeshResource->GetStaticMeshData().IndexCount
	};
}
