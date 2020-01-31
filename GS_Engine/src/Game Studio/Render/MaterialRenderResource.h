#pragma once

#include "RenderResource.h"
#include "Containers/Array.hpp"
#include "RAPI/Texture.h"

struct MaterialRenderResourceCreateInfo : public RenderResourceCreateInfo
{
	class Material* ParentMaterial = nullptr;
	Array<class RAPI::Texture*, 8> textures;
};

class MaterialRenderResource : public RenderResource
{
	class Material* referenceMaterial = nullptr;

	Array<class RAPI::Texture*, 8> textures;

public:
	explicit MaterialRenderResource(const MaterialRenderResourceCreateInfo& MRRCI_);
};
