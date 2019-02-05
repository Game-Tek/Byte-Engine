#include "StaticMeshRenderProxy.h"

#include "Application.h"

StaticMeshRenderProxy::StaticMeshRenderProxy()
{
	Application::GetResourceManager()->GetResource();
}

StaticMeshRenderProxy::~StaticMeshRenderProxy()
{
}