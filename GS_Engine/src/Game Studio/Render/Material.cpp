#include "Material.h"

#include "Resources/MaterialResource.h"
#include "Application/Application.h"

Material::Material(const FString& _Name) : materialMaterialResource(GS::Application::Get()->GetResourceManager()->GetResource<MaterialResource>(_Name))
{
}

Material::~Material()
{
	GS::Application::Get()->GetResourceManager()->ReleaseResource(materialMaterialResource);
}

const char* Material::GetMaterialName() const
{
	return materialMaterialResource->GetMaterialData().GetResourceName().c_str();
}

void Material::GetRenderingCode(FVector<ShaderInfo>& shaders_) const
{
	shaders_.resize(2);
	
	shaders_[0].Type = ShaderType::VERTEX_SHADER;
	shaders_[0].ShaderCode = &const_cast<FString&>(materialMaterialResource->GetMaterialData().GetVertexShaderCode());
	shaders_[1].Type = ShaderType::FRAGMENT_SHADER;
	shaders_[1].ShaderCode = &const_cast<FString&>(materialMaterialResource->GetMaterialData().GetFragmentShaderCode());
}


void Material::SetParameter(const Id& parameter_name_, ShaderDataTypes data_type_, void* data_)
{
	for(auto& e : parameters)
	{
		if(e.ParameterName == parameter_name_)
		{
			memcpy(e.Data, data_, ShaderDataTypesSize(data_type_));

			return;
		}
	}

	GS_THROW("No parameter with such name!")
}

bool Material::GetHasTransparency() const
{
	return false;/*return materialMaterialResource->GetHasTransparency();*/
}

bool Material::GetIsTwoSided() const { return false; }/*return materialMaterialResource->GetIsTwoSided();*/
