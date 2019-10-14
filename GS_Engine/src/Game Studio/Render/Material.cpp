#include "Material.h"

#include "Resources/MaterialResource.h"
#include "Application/Application.h"

Material::Material(const FString& _Name) : materialMaterialResource(GS::Application::Get()->GetResourceManager()->GetResource<MaterialResource>(_Name))
{
}

const char* Material::GetMaterialName() const
{
	return materialMaterialResource->GetData()->GetResourceName();
}

void Material::GetRenderingCode(char** _VS, char** _FS) const
{
	*_VS = static_cast<MaterialResource::MaterialData*>(materialMaterialResource->GetData())->GetVertexShaderCode();
	*_FS = static_cast<MaterialResource::MaterialData*>(materialMaterialResource->GetData())->GetFragmentShaderCode();
}
