#include "StaticMesh.h"

#include "ResourceManager.h"

#include "Logger.h"

StaticMesh::StaticMesh(const std::string & Path)
{
	MeshData = ResourceManager::GetAsset<StaticMeshResource>(Path);

	GS_LOG_MESSAGE("Loaded static mesh %s, ", Path.c_str())
}