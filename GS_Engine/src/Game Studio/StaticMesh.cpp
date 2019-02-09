#include "StaticMesh.h"

#include "ResourceManager.h"

#include "Logger.h"

#include "Application.h"

StaticMesh::StaticMesh(const std::string & StaticMeshAsset) : RenderProxy(this), MeshResource(GS::Application::GetResourceManagerInstance()->GetResource<StaticMeshResource>(StaticMeshAsset))
{
	GS_LOG_MESSAGE("Loaded static mesh %s, ", StaticMeshAsset.c_str())
}

StaticMesh::~StaticMesh()
{

}