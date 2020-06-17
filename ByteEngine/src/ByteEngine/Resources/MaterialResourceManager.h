#pragma once

#include "SubResourceManager.h"
#include "ResourceData.h"
#include <GTSL/Id.h>

struct MaterialResourceData final : ResourceHandle
{
	float Roughness;
};

class MaterialResourceManager final : public SubResourceManager
{
public:
	MaterialResourceManager() : SubResourceManager("Material")
	{
	}
};
