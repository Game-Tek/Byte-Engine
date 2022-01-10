#pragma once

#include "ResourceManager.h"

#include <GTSL/Extent.h>
#include <GAL/RenderCore.h>
#include <GTSL/File.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/MappedFile.hpp>

#include "ByteEngine/Game/ApplicationManager.h"

class TextureResourceManager final : public ResourceManager
{
public:
	TextureResourceManager(const InitializeInfo&);
	~TextureResourceManager();
	
	struct TextureInfo : SData {
		DEFINE_MEMBER(GTSL::Extent3D, Extent);
		DEFINE_MEMBER(GAL::FormatDescriptor, Format);
	};
	
	template<typename... ARGS>
	void LoadTextureInfo(ApplicationManager* gameInstance, Id textureName, DynamicTaskHandle<TextureInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"loadTextureInfo", {}, &TextureResourceManager::loadTextureInfo<ARGS...>, {}, {}, GTSL::MoveRef(textureName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
	template<typename... ARGS>
	void LoadTexture(ApplicationManager* gameInstance, TextureInfo textureInfo, GTSL::Range<byte*> buffer, DynamicTaskHandle<TextureInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"loadTexture", {}, &TextureResourceManager::loadTexture<ARGS...>, {}, {}, GTSL::MoveRef(textureInfo), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:

	template<typename... ARGS>
	void loadTextureInfo(TaskInfo taskInfo, Id textureName, DynamicTaskHandle<TextureInfo, ARGS...> dynamicTaskHandle, ARGS... args) {
		TextureInfo textureInfo;
		resource_files_.LoadEntry(textureName, textureInfo);
		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadTexture(TaskInfo taskInfo, TextureInfo textureInfo, GTSL::Range<byte*> buffer, DynamicTaskHandle<TextureInfo, ARGS...> dynamicTaskHandle, ARGS... args) {
		resource_files_.LoadData(textureInfo, buffer);
		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	ResourceFiles resource_files_;
};