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
	
	struct TextureData
	{
		uint32 ImageSize = 0;
		GAL::Dimension Dimensions;
		GTSL::Extent3D Extent;
		GAL::TextureFormat Format;
	};

	struct TextureInfoSerialize : TextureData
	{
		uint32 ByteOffset = 0;

		template<class ALLOCATOR>
		friend void Insert(const TextureInfoSerialize& textureInfoSerialize, GTSL::Buffer<ALLOCATOR>& buffer)
		{
			Insert(textureInfoSerialize.ByteOffset, buffer);
			Insert(textureInfoSerialize.ImageSize, buffer);
			Insert(textureInfoSerialize.Dimensions, buffer);
			Insert(textureInfoSerialize.Extent, buffer);
			Insert(textureInfoSerialize.Format, buffer);
		}

		template<class ALLOCATOR>
		friend void Extract(TextureInfoSerialize& textureInfoSerialize, GTSL::Buffer<ALLOCATOR>& buffer)
		{
			Extract(textureInfoSerialize.ByteOffset, buffer);
			Extract(textureInfoSerialize.ImageSize, buffer);
			Extract(textureInfoSerialize.Dimensions, buffer);
			Extract(textureInfoSerialize.Extent, buffer);
			Extract(textureInfoSerialize.Format, buffer);
		}
	};

	struct TextureInfo : TextureInfoSerialize
	{
		TextureInfo() = default;
		TextureInfo(const TextureInfoSerialize& textureInfoSerialize) : TextureInfoSerialize(textureInfoSerialize) {}
		
		Id Name;
	};
	
	template<typename... ARGS>
	void LoadTextureInfo(GameInstance* gameInstance, Id textureName, DynamicTaskHandle<TextureResourceManager*, TextureInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadTextureInfo = [](TaskInfo taskInfo, TextureResourceManager* resourceManager, Id textureName, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			auto textureInfoSerialize = resourceManager->textureInfos.At(textureName());

			TextureInfo textureInfo(textureInfoSerialize);
			textureInfo.Name = textureName;
			
			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(textureInfo), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask("loadTextureInfo", Task<TextureResourceManager*, Id, decltype(dynamicTaskHandle), ARGS...>::Create(loadTextureInfo), {}, this, GTSL::MoveRef(textureName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	using Texture = TextureInfo;
	
	template<typename... ARGS>
	void LoadTexture(GameInstance* gameInstance, TextureInfo textureInfo, GTSL::Range<byte*> buffer, DynamicTaskHandle<TextureResourceManager*, TextureInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadTexture = [](TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureInfo textureInfo, GTSL::Range<byte*> buffer, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			resourceManager->packageFile.SetPointer(textureInfo.ByteOffset, GTSL::File::MoveFrom::BEGIN);
			resourceManager->packageFile.ReadFromFile(GTSL::Range<byte*>(textureInfo.ImageSize, buffer.begin()));
			
			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(textureInfo), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask("loadTexture", Task<TextureResourceManager*, TextureInfo, GTSL::Range<byte*>, decltype(dynamicTaskHandle), ARGS...>::Create(loadTexture), {}, this, GTSL::MoveRef(textureInfo), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}


private:
	GTSL::File packageFile, indexFile;
	GTSL::FlatHashMap<TextureInfoSerialize, BE::PersistentAllocatorReference> textureInfos;
	GTSL::Mutex fileLock;
};