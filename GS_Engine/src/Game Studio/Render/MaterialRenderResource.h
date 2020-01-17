#pragma once

#include "RenderResource.h"
#include "Containers/Array.hpp"

struct MaterialRenderResourceCreateInfo : public RenderResourceCreateInfo
{
	class Material* ParentMaterial = nullptr;
	Array<class Texture*, 8> textures;
};

class MaterialRenderResource : public RenderResource
{
	class Material* referenceMaterial = nullptr;
	
	Array<class Texture*, 8> textures;

public:
	explicit MaterialRenderResource(const MaterialRenderResourceCreateInfo& MRRCI_);
};
