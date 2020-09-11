#pragma once

#include <GTSL/StaticString.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Vector2.h>


#include "RenderTypes.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/FontResourceManager.h"

class RenderSystem;

class TextSystem : public System
{
public:
	TextSystem() : System("TextSystem") {}

	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	FontResourceManager::ImageFont& GetFont() { return *font; }
	int32 GetAtlasTextureIndex() const { return 0; }

	struct AddTextInfo
	{
		GameInstance* GameInstance;
		FontResourceManager* FontResourceManager;
		GTSL::Vector2 Position;
		GTSL::StaticString<64> Text;
		RenderSystem* RenderSystem;
		ComponentReference Material;
	};
	ComponentReference AddText(const AddTextInfo& addTextInfo);
	
	struct Text
	{
		GTSL::Vector2 Position;
		GTSL::StaticString<64> String;
	};
	GTSL::Ranger<const Text> GetTexts() const { return components; }
	
private:
	Vector<Text> components;

	struct LoadInfo
	{
		LoadInfo(uint32 component, ComponentReference material, Buffer buffer, RenderSystem* renderSystem, RenderAllocation allocation) : Component(component), Material(material),
		Buffer(buffer), RenderSystem(renderSystem), Allocation(allocation) {}
		
		uint32 Component;
		ComponentReference Material;
		Buffer Buffer;
		RenderSystem* RenderSystem;
		RenderAllocation Allocation;
	};
	
	struct AtlasData
	{
		Texture Texture;
		TextureView TextureView;
		TextureSampler TextureSampler;

		RenderAllocation Allocation;
	};
	GTSL::KeepVector<AtlasData, BE::PersistentAllocatorReference> textures;

	FontResourceManager::ImageFont* font;
	
	void onFontLoad(TaskInfo, FontResourceManager::OnFontLoadInfo);
};
