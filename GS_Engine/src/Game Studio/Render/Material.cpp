#include "Material.h"

#include "Resources/MaterialResource.h"
#include "Application/Application.h"

using namespace RAPI;

Material::Material(const FString& _Name) : materialMaterialResource(
	GS::Application::Get()->GetResourceManager()->GetResource<MaterialResource>(_Name))
{
}

Material::~Material()
{
	GS::Application::Get()->GetResourceManager()->ReleaseResource(materialMaterialResource);
}

Id Material::GetMaterialType() const { return materialMaterialResource->GetMaterialData().GetResourceName().c_str(); }

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
	for (auto& e : parameters)
	{
		if (e.ParameterName == parameter_name_)
		{
			memcpy(e.Data, data_, ShaderDataTypesSize(data_type_));

			return;
		}
	}

	GS_THROW("No parameter with such name!")
}

void Material::SetTexture(const Id& textureName, Texture* texturePointer)
{
	textures[textureName.GetID()] = texturePointer;
	textures.resize(1);
}

bool Material::GetHasTransparency() const { return materialMaterialResource->GetMaterialData().HasTransparency; }

bool Material::GetIsTwoSided() const { return materialMaterialResource->GetMaterialData().IsTwoSided; }
