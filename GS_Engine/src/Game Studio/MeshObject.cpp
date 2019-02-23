#include "MeshObject.h"

#include "MeshRenderProxy.h"

MeshObject::MeshObject(MeshRenderProxy * RenderProxy) : RenderProxy(RenderProxy)
{
}

MeshObject::~MeshObject()
{
	delete RenderProxy;
}
