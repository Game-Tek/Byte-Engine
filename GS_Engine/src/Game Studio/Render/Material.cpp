#include "Material.h"

#include "Resources/MaterialResource.h"

const char* Material::GetMaterialName() const
{
	return materialMaterialResource->GetData()->GetResourceName();
}
