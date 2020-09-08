#include "TextSystem.h"

#include "MaterialSystem.h"
#include "ByteEngine/Game/GameInstance.h"

void TextSystem::Initialize(const InitializeInfo& initializeInfo)
{
	components.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());

	{
		MaterialSystem::AddRenderGroupInfo addRenderGroupInfo;
		addRenderGroupInfo.Name = "TextSystem";
		addRenderGroupInfo.Bindings.EmplaceBack();
		addRenderGroupInfo.Bindings.back().EmplaceBack(BindingType::STORAGE_BUFFER_DYNAMIC);

		addRenderGroupInfo.Range.EmplaceBack();
		addRenderGroupInfo.Range.back().EmplaceBack(512*512);

		addRenderGroupInfo.Size.EmplaceBack();
		addRenderGroupInfo.Size.back().EmplaceBack(1024*1024);
		
		//addRenderGroupInfo.Data.EmplaceBack();
		//addRenderGroupInfo.Data.back().EmplaceBack(GAL::ShaderDataType::FLOAT4);
		//addRenderGroupInfo.Data.back().EmplaceBack(GAL::ShaderDataType::FLOAT4); //8 floats, 32 bytes
		
		initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem")->AddRenderGroup(initializeInfo.GameInstance, addRenderGroupInfo);
	}
}

void TextSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

System::ComponentReference TextSystem::AddText(const AddTextInfo& addTextInfo)
{
	Text text;
	text.Position = addTextInfo.Position;
	text.String = addTextInfo.Text;

	return components.EmplaceBack(text);
}
