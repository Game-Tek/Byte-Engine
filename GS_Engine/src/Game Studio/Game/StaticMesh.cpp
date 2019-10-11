#include "StaticMesh.h"

#include "Resources/StaticMeshResource.h"
#include "Application/Application.h"

StaticMesh::StaticMesh(const FString& _Name) : staticMeshResource(GS::Application::Get()->GetResourceManager()->GetResource<StaticMeshResource>(_Name))
{
}

StaticMesh::~StaticMesh()
{
	GS::Application::Get()->GetResourceManager()->ReleaseResource(staticMeshResource);
}

Model* StaticMesh::GetModel() const { return staticMeshResource->GetData()->GetModel(); }
