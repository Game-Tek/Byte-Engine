#include "StaticMesh.h"

#include "ResourceManager.h"

#include "Logger.h"

#include "Application.h"

#include "StaticMeshRenderProxy.h"

StaticMesh::StaticMesh(const std::string & StaticMeshAsset) : MeshResource(GS::Application::GetResourceManagerInstance()->GetResource<StaticMeshResource>(StaticMeshAsset))
{
	RenderProxy = new StaticMeshRenderProxy(MeshResource);
}

StaticMesh::~StaticMesh()
{

}