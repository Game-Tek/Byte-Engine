#include "StaticMesh.h"

#include "Resources/StaticMeshResource.h"
#include "Resources/StaticMeshResourceManager.h"

StaticMesh::StaticMesh(const FString& _Name) : staticMeshResource(StaticMeshResourceManager::Get().GetResource(_Name))
{
}

StaticMesh::~StaticMesh()
{
	StaticMeshResourceManager::Get().ReleaseResource(staticMeshResource);
}

Model* StaticMesh::GetModel() const { return staticMeshResource->GetModel(); }
