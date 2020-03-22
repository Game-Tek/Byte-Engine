#include "Material.h"

#include "Application/Application.h"
#include "Resources/MaterialResourceManager.h"

using namespace RAPI;

Material::Material(const FString& _Name) : materialMaterialResource(BE::Application::Get()->GetResourceManager()->TryGetResource(_Name, "Material"))
{
	//static_cast<MaterialResourceData*>(BE::Application::Get()->GetResourceManager()->GetResource(materialMaterialResource))->Roughness;
}

Material::~Material()
{
	BE::Application::Get()->GetResourceManager()->ReleaseResource(materialMaterialResource);
}

Id64 Material::GetMaterialType() const { return materialMaterialResource.GetName(); }

//void Material::GetRenderingCode(FVector<ShaderInfo>& shaders_) const
//{
//	shaders_.resize(2);
//
//	shaders_[0].Type = ShaderType::VERTEX_SHADER;
//	shaders_[0].ShaderCode = &const_cast<FString&>(materialMaterialResource->GetMaterialData().GetVertexShaderCode());
//	shaders_[1].Type = ShaderType::FRAGMENT_SHADER;
//	shaders_[1].ShaderCode = &const_cast<FString&>(materialMaterialResource->GetMaterialData().GetFragmentShaderCode());
//}
//
//
//void Material::SetParameter(const Id64& parameter_name_, ShaderDataTypes data_type_, void* data_)
//{
//	for (auto& e : parameters)
//	{
//		if (e.ParameterName == parameter_name_)
//		{
//			memcpy(e.Data, data_, ShaderDataTypesSize(data_type_));
//
//			return;
//		}
//	}
//
//	BE_THROW("No parameter with such name!")
//}
//
//void Material::SetTexture(const Id64& textureName, Texture* texturePointer)
//{
//	textures[textureName.GetID()] = texturePointer;
//	textures.resize(1);
//}
//
//bool Material::GetHasTransparency() const { return materialMaterialResource->GetMaterialData().HasTransparency; }
//
//bool Material::GetIsTwoSided() const { return materialMaterialResource->GetMaterialData().IsTwoSided; }
//