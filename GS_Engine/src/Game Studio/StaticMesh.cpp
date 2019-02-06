#include "StaticMesh.h"

#include "ResourceManager.h"

#include "Logger.h"

StaticMesh::StaticMesh(const std::string & Path) : RenderProxy(this)
{
	MeshResource = ResourceManager::GetResource<StaticMeshResource>(Path);

	GS_LOG_MESSAGE("Loaded static mesh %s, ", Path.c_str())
}