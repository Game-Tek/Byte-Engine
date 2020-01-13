#pragma once

#include "RenderResource.h"
#include "Containers/Array.hpp"

struct MaterialRenderResourceCreateInfo
{
	class Material* ParentMaterial = nullptr;
	Array<class Texture*, 8> textures;
};

class MaterialRenderResource : public RenderResource
{
	class Material* referenceMaterial = nullptr;
	
	Array<class Texture*, 8> textures;

public:
	explicit MaterialRenderResource(const MaterialRenderResourceCreateInfo& MRRCI_) : RenderResource(), referenceMaterial(MRRCI_.ParentMaterial), textures(MRRCI_.textures)
	{
		
	}
};
