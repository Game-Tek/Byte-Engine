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
		addRenderGroupInfo.Bindings.EmplaceBack();
		addRenderGroupInfo.Bindings.back().EmplaceBack(BindingType::STORAGE_BUFFER_DYNAMIC);
		
		addRenderGroupInfo.Data.EmplaceBack();
		addRenderGroupInfo.Data.back().EmplaceBack(GAL::ShaderDataType::FLOAT4);
		addRenderGroupInfo.Data.back().EmplaceBack(GAL::ShaderDataType::FLOAT4); //8 floats, 32 bytes
		
		initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem")->AddRenderGroup(initializeInfo.GameInstance, addRenderGroupInfo);
	}

	renderingFont = BE::Application::Get()->GetResourceManager<FontResourceManager>("FontResourceManager")->GetFont(GTSL::StaticString<8>("Rage"));

	//auto& glyph = renderingFont.Glyphs[renderingFont.GlyphMap['5']];
	//
	//BE_LOG_MESSAGE("Num contours ", glyph.NumContours)
	//BE_LOG_MESSAGE("Left side bearing ", glyph.LeftSideBearing)
	//for(auto& e : glyph.PathList)
	//{
	//	BE_LOG_MESSAGE("Path\n")
	//
	//	for(auto& b : e.Curves)
	//	{
	//		BE_LOG_MESSAGE("P0: X = ", b.p0.X, " Y = ", b.p0.Y)
	//		BE_LOG_MESSAGE("P1: X = ", b.p1.X, " Y = ", b.p1.Y)
	//		BE_LOG_MESSAGE("P2: X = ", b.p2.X, " Y = ", b.p2.Y)
	//	}
	//}
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
