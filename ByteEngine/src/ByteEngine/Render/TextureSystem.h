#pragma once

#include "RenderTypes.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

class RenderSystem;

class TextureSystem : public System
{
public:

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
		LoadInfo(uint32 component, Buffer buffer) : Component(component), Buffer(buffer)
		{
		}

		uint32 Component;
		Buffer Buffer;
	};
	void onTextureLoad(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo onTextureLoadInfo);

	ComponentReference component = 0;
};
