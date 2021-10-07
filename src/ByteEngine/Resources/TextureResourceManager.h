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
	TextureResourceManager();
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
	void LoadTextureInfo(ApplicationManager* gameInstance, Id textureName, DynamicTaskHandle<TextureResourceManager*, TextureInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadTextureInfo = [](TaskInfo taskInfo, TextureResourceManager* resourceManager, Id textureName, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			if constexpr (BE_DEBUG) {
				if (!resourceManager->textureInfos.Find(textureName)) {
					resourceManager->getLogger()->PrintObjectLog(resourceManager, BE::Logger::VerbosityLevel::FATAL, u8"Texture with name ", GTSL::StringView(textureName), u8" could not be found. ", BE::FIX_OR_CRASH_STRING);
					return;
				}
			}

			auto textureInfoSerialize = resourceManager->textureInfos.At(textureName);

			TextureInfo textureInfo(textureName, textureInfoSerialize);
			
			taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask(u8"loadTextureInfo", Task<TextureResourceManager*, Id, decltype(dynamicTaskHandle), ARGS...>::Create(loadTextureInfo), {}, this, GTSL::MoveRef(textureName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
	template<typename... ARGS>
	void LoadTexture(ApplicationManager* gameInstance, TextureInfo textureInfo, GTSL::Range<byte*> buffer, DynamicTaskHandle<TextureResourceManager*, TextureInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadTexture = [](TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureInfo textureInfo, GTSL::Range<byte*> buffer, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			GTSL::MemCopy(textureInfo.GetTextureSize(), resourceManager->mappedFile.GetData(), buffer.begin());
			taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask(u8"loadTexture", Task<TextureResourceManager*, TextureInfo, GTSL::Range<byte*>, decltype(dynamicTaskHandle), ARGS...>::Create(loadTexture), {}, this, GTSL::MoveRef(textureInfo), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File indexFile;
	GTSL::MappedFile mappedFile;
	GTSL::HashMap<Id, TextureDataSerialize, BE::PersistentAllocatorReference> textureInfos;
};