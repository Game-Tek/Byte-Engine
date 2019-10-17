#include "Material.h"

#include "Resources/MaterialResource.h"
#include "Application/Application.h"

Material::Material(const FString& _Name) : materialMaterialResource(GS::Application::Get()->GetResourceManager()->GetResource<MaterialResource>(_Name))
{
}

const char* Material::GetMaterialName() const
{
	return materialMaterialResource->GetData()->GetResourceName().c_str();
}

void Material::GetRenderingCode(ShaderInfo& _VS, ShaderInfo& _FS) const
{
	_VS.Type = ShaderType::VERTEX_SHADER;
	_VS.ShaderCode = &static_cast<MaterialResource::MaterialData*>(materialMaterialResource->GetData())->GetVertexShaderCode();
	_FS.Type = ShaderType::FRAGMENT_SHADER;
	_FS.ShaderCode = &static_cast<MaterialResource::MaterialData*>(materialMaterialResource->GetData())->GetFragmentShaderCode();
}
