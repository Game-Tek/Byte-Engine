#include "TextSystem.h"


#include "MaterialSystem.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"

void TextSystem::Initialize(const InitializeInfo& initializeInfo)
{
	components.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());

	{
		MaterialSystem::AddRenderGroupInfo addRenderGroupInfo;
		addRenderGroupInfo.Name = "TextSystem";
		addRenderGroupInfo.Bindings.EmplaceBack(); addRenderGroupInfo.Bindings.back().EmplaceBack(BindingType::UNIFORM_BUFFER_DYNAMIC);
		addRenderGroupInfo.Data.EmplaceBack(); addRenderGroupInfo.Data.back().EmplaceBack(GAL::ShaderDataType::MAT4);
		
		initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem")->AddRenderGroup(initializeInfo.GameInstance, addRenderGroupInfo);
	}

	renderingFont = BE::Application::Get()->GetResourceManager<FontResourceManager>("FontResourceManager")->GetFont(GTSL::StaticString<8>("Rage"));

	//renderingFont.Glyphs[renderingFont.GlyphMap['A']];
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
