#pragma once

#include "ResourceManager.h"

#include <GTSL/Extent.h>
#include <GAL/RenderCore.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>

#include "ByteEngine/Game/GameInstance.h"

class TextureResourceManager final : public ResourceManager
{
public:
	TextureResourceManager();
	~TextureResourceManager();
	
	struct TextureData : Data
	{
		GAL::Dimension Dimensions;
		GTSL::Extent3D Extent;
		GAL::FormatDescriptor Format;
	};
	
	struct TextureDataSerialize : DataSerialize<TextureData>
	{
		INSERT_START(TextureDataSerialize)
		{
			INSERT_BODY
			Insert(insertInfo.Dimensions, buffer);
			Insert(insertInfo.Extent, buffer);
			Insert(insertInfo.Format, buffer);
		}

		EXTRACT_START(TextureDataSerialize)
		{
			EXTRACT_BODY
			Extract(extractInfo.Dimensions, buffer);
			Extract(extractInfo.Extent, buffer);
			Extract(extractInfo.Format, buffer);
		}
	};

	struct TextureInfo : Info<TextureDataSerialize>
	{
		DECL_INFO_CONSTRUCTOR(TextureInfo, Info<TextureDataSerialize>)
		
		uint32 GetTextureSize()
		{
			return Format.BitDepth * Format.ComponentCount * Extent.Width * Extent.Height * Extent.Depth;
		}
	};
	
	template<typename... ARGS>
	void LoadTextureInfo(GameInstance* gameInstance, Id textureName, DynamicTaskHandle<TextureResourceManager*, TextureInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadTextureInfo = [](TaskInfo taskInfo, TextureResourceManager* resourceManager, Id textureName, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			auto textureInfoSerialize = resourceManager->textureInfos.At(textureName());

			TextureInfo textureInfo(textureName, textureInfoSerialize);
			
			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask("loadTextureInfo", Task<TextureResourceManager*, Id, decltype(dynamicTaskHandle), ARGS...>::Create(loadTextureInfo), {}, this, GTSL::MoveRef(textureName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
	template<typename... ARGS>
	void LoadTexture(GameInstance* gameInstance, TextureInfo textureInfo, GTSL::Range<byte*> buffer, DynamicTaskHandle<TextureResourceManager*, TextureInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadTexture = [](TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureInfo textureInfo, GTSL::Range<byte*> buffer, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			resourceManager->packageFiles[resourceManager->getThread()].SetPointer(textureInfo.ByteOffset, GTSL::File::MoveFrom::BEGIN);
			resourceManager->packageFiles[resourceManager->getThread()].ReadFromFile(GTSL::Range<byte*>(textureInfo.GetTextureSize(), buffer.begin()));
			
			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask("loadTexture", Task<TextureResourceManager*, TextureInfo, GTSL::Range<byte*>, decltype(dynamicTaskHandle), ARGS...>::Create(loadTexture), {}, this, GTSL::MoveRef(textureInfo), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File indexFile;
	GTSL::FlatHashMap<TextureDataSerialize, BE::PersistentAllocatorReference> textureInfos;
};