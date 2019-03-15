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

StaticMesh::StaticMesh() : MeshRenderProxy(new StaticMeshRenderProxy(this, Vertices, sizeof(Vertices), Indices, 6))
{
}

StaticMesh::StaticMesh(const String & StaticMeshAsset) : MeshResource(GS::Application::Get()->GetResourceManagerInstance()->GetResource(StaticMeshAsset)), MeshRenderProxy(new StaticMeshRenderProxy(this, MeshResource->GetVertexArray(), MeshResource->GetDataSize(), MeshResource->GetIndexArray(), MeshResource->GetMeshIndexCount(0)))
{
}

StaticMesh::~StaticMesh()
{
	delete MeshRenderProxy;
	delete MeshResource;
}