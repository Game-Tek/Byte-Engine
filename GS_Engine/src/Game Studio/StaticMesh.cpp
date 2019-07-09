#include "StaticMesh.h"

#include "ResourceManager.h"

#include "Application.h"

#include "StaticMeshResource.h"

#include "FString.h"

Vertex Vertices[] = { { { 50.0f, -100.f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } }
					, { { -50.0f, -50.0f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 1.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } }
					, { { 0.0f, 50.0f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 1.0f, 1.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } } };

unsigned int Indices[] = { 0, 1, 2, 2, 3, 0, 4, 0, 3 };

StaticMesh::StaticMesh()
{
}

StaticMesh::StaticMesh(const String & StaticMeshAsset) : MeshResource(GS::Application::Get()->GetResourceManagerInstance()->GetResource(StaticMeshAsset))
{
}

StaticMesh::~StaticMesh()
{
}