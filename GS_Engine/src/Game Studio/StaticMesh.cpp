#include "StaticMesh.h"

#include "ResourceManager.h"

#include "Application.h"

#include "StaticMeshResource.h"

#include "StaticMeshRenderProxy.h"

#include "String.h"

Vertex Vertices[] = { { { -0.5f, -0.5f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } }
					, { { -0.5f, 0.5f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 1.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } }
					, { { 0.5f, 0.5f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 1.0f, 1.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } }
					, { { 0.5f, -0.5f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 1.0f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } } };

unsigned int Indices[] = { 0, 1, 2, 2, 3, 0 };

StaticMesh::StaticMesh() : MeshObject(new StaticMeshRenderProxy(Vertices, sizeof(Vertices), Indices, 6))
{
}

StaticMesh::StaticMesh(const String & StaticMeshAsset) : MeshObject(new StaticMeshRenderProxy(this)), MeshResource(GS::Application::Get()->GetResourceManagerInstance()->GetResource<StaticMeshResource>(StaticMeshAsset))
{
}

StaticMesh::~StaticMesh()
{
	delete MeshResource;
}