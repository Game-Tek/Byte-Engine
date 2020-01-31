#pragma once

#include "RenderResource.h"

struct MeshRenderResourceCreateInfo
{
	class RAPI::RenderMesh* Mesh = nullptr;
};

class MeshRenderResource : public RenderResource
{
	friend class Renderer;

	class RAPI::RenderMesh* mesh = nullptr;

public:
	explicit MeshRenderResource(const MeshRenderResourceCreateInfo& MRRCI_) : RenderResource(), mesh(MRRCI_.Mesh)
	{
	}
};
