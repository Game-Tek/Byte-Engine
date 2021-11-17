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
	
	struct TextureData : Data
	{
		GTSL::Extent3D Extent;
		GAL::FormatDescriptor Format;
	};
	
	struct TextureDataSerialize : DataSerialize<TextureData>
	{
		INSERT_START(TextureDataSerialize)
		{
			INSERT_BODY
			Insert(insertInfo.Extent, buffer);
			Insert(insertInfo.Format, buffer);
		}

		EXTRACT_START(TextureDataSerialize)
		{
			EXTRACT_BODY
			Extract(extractInfo.Extent, buffer);
			Extract(extractInfo.Format, buffer);
		}
	};

	struct TextureInfo : Info<TextureDataSerialize>
	{
		DECL_INFO_CONSTRUCTOR(TextureInfo, Info<TextureDataSerialize>)
		
		uint32 GetTextureSize()
		{
			return Format.GetSize() * Extent.Width * Extent.Height * Extent.Depth;
		}
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
	void loadTextureInfo(TaskInfo taskInfo, Id textureName, DynamicTaskHandle<TextureInfo, ARGS...> dynamicTaskHandle, ARGS... args)
	{
		if constexpr (BE_DEBUG) {
			if (!textureInfos.Find(textureName)) {
				getLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::FATAL, u8"Texture with name ", GTSL::StringView(textureName), u8" could not be found. ", BE::FIX_OR_CRASH_STRING);
				return;
			}
		}

		auto textureInfoSerialize = textureInfos.At(textureName);

		TextureInfo textureInfo(textureName, textureInfoSerialize);

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadTexture(TaskInfo taskInfo, TextureInfo textureInfo, GTSL::Range<byte*> buffer, DynamicTaskHandle<TextureInfo, ARGS...> dynamicTaskHandle, ARGS... args)
	{
		GTSL::MemCopy(textureInfo.GetTextureSize(), mappedFile.GetData(), buffer.begin());
		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	GTSL::File indexFile;
	GTSL::MappedFile mappedFile;
	GTSL::HashMap<Id, TextureDataSerialize, BE::PersistentAllocatorReference> textureInfos;
};