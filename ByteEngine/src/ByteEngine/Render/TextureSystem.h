#pragma once

#include "RenderTypes.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

class RenderSystem;

class TextureSystem : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	struct CreateTextureInfo
	{
		Id TextureName;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager = nullptr;
	};
	ComponentReference CreateTexture(const CreateTextureInfo& info);
	
private:
	struct LoadInfo
	{
		LoadInfo(uint32 component, Buffer buffer, RenderSystem* renderSystem, RenderAllocation renderAllocation) : Component(component), Buffer(buffer), RenderSystem(renderSystem), RenderAllocation(renderAllocation)
		{
		}

		uint32 Component;
		Buffer Buffer;
		RenderSystem* RenderSystem;
		RenderAllocation RenderAllocation;
	};
	void onTextureLoad(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo onTextureLoadInfo);

	ComponentReference component = 0;

	struct TextureComponent
	{
		Texture Texture;
		TextureView TextureView;
		RenderAllocation Allocation;
	};
	Vector<TextureComponent> textures;
};
