#include "TextureSystem.h"

#include "RenderSystem.h"
#include "RenderTypes.h"

System::ComponentReference TextureSystem::CreateTexture(const CreateTextureInfo& info)
{	
	TextureResourceManager::TextureLoadInfo textureLoadInfo;
	textureLoadInfo.GameInstance = info.GameInstance;
	textureLoadInfo.Name = info.TextureName;

	textureLoadInfo.OnTextureLoadInfo = GTSL::Delegate<void(TaskInfo, TextureResourceManager::OnTextureLoadInfo)>::Create<TextureSystem, &TextureSystem::onTextureLoad>(this);

	const GTSL::Array<TaskDependency, 6> loadTaskDependencies{ { "TextureSystem", AccessType::READ_WRITE }, { "RenderSystem", AccessType::READ_WRITE } };
	
	textureLoadInfo.ActsOn = loadTaskDependencies;

	{
		Buffer::CreateInfo scratchBufferCreateInfo;
		scratchBufferCreateInfo.RenderDevice = info.RenderSystem->GetRenderDevice();
		scratchBufferCreateInfo.Size = info.TextureResourceManager->GetTextureSize(info.TextureName);
		scratchBufferCreateInfo.BufferType = BufferType::TRANSFER_SOURCE;
		
		void* loadInfo;
		GTSL::New<LoadInfo>(&loadInfo, GetPersistentAllocator(), component, Buffer(scratchBufferCreateInfo), info.RenderSystem);

		textureLoadInfo.UserData = DYNAMIC_TYPE(LoadInfo, loadInfo);
	}
	
	info.TextureResourceManager->LoadTexture(textureLoadInfo);
	
	return component++;
}

void TextureSystem::onTextureLoad(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo onTextureLoadInfo)
{
	auto* loadInfo = DYNAMIC_CAST(LoadInfo, onTextureLoadInfo.UserData);

	TextureComponent textureComponent;
	
	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.Uses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
		textureCreateInfo.Dimensions = ConvertDimension(onTextureLoadInfo.Dimensions);
		textureCreateInfo.SourceFormat = ConvertFormat(onTextureLoadInfo.TextureFormat);
		textureCreateInfo.Extent = onTextureLoadInfo.Extent;
		textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
		textureCreateInfo.MipLevels = 1;

		textureComponent.Texture = Texture(textureCreateInfo);
	}

	{
		DeviceMemory deviceMemory;

		RenderSystem::AllocateLocalTextureMemoryInfo allocationInfo;
		allocationInfo.DeviceMemory = &deviceMemory;
		allocationInfo.Allocation = &textureComponent.Allocation;
		allocationInfo.Texture = textureComponent.Texture;
		
		loadInfo->RenderSystem->AllocateLocalTextureMemory(allocationInfo);
	}
	
	{
		RenderSystem::TextureCopyData textureCopyData;
		textureCopyData.DestinationTexture = textureComponent.Texture;
		textureCopyData.SourceBuffer = loadInfo->Buffer;
		textureCopyData.Allocation = textureComponent.Allocation;
		textureCopyData.Layout = TextureLayout::TRANSFER_DST;
		textureCopyData.Extent = onTextureLoadInfo.Extent;

		loadInfo->RenderSystem->AddTextureCopy(textureCopyData);
	}
	
	textures.Insert(loadInfo->Component, textureComponent);
}
