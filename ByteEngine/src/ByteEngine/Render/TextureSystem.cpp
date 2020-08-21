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
		GTSL::New<LoadInfo>(&loadInfo, GetPersistentAllocator(), component, Buffer(scratchBufferCreateInfo));

		textureLoadInfo.UserData = DYNAMIC_TYPE(LoadInfo, loadInfo);
	}
	
	info.TextureResourceManager->LoadTexture(textureLoadInfo);
	
	return component++;
}

void TextureSystem::onTextureLoad(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo onTextureLoadInfo)
{
	auto* loadInfo = DYNAMIC_CAST(LoadInfo, onTextureLoadInfo.UserData);
	RenderSystem* renderSystem;

	TextureComponent textureComponent;

	{
		Buffer::CreateInfo bufferCreateInfo;
		bufferCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		bufferCreateInfo.Size = onTextureLoadInfo.DataBuffer.ElementCount();
		bufferCreateInfo.BufferType = BufferType::TRANSFER_DESTINATION; // | TEXTURE;

		textureComponent.TextureBuffer = Buffer(bufferCreateInfo);
	}

	{
		DeviceMemory deviceMemory;
		
		RenderSystem::BufferLocalMemoryAllocationInfo allocationInfo;
		allocationInfo.DeviceMemory = &deviceMemory;
		allocationInfo.Allocation = &textureComponent.Allocation;
		renderSystem->AllocateLocalBufferMemory(allocationInfo);
	}

	{
		RenderSystem::BufferCopyData bufferCopyData;
		
		//renderSystem->AddBufferCopy();
	}
	
	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.ImageUses = ImageUse::TRANSFER_DESTINATION | ImageUse::SAMPLE;
		textureCreateInfo.Dimensions = ConvertDimension(onTextureLoadInfo.Dimensions);
		textureCreateInfo.SourceFormat = ConvertFormat(onTextureLoadInfo.TextureFormat);
		textureCreateInfo.Extent = onTextureLoadInfo.Extent;
		textureCreateInfo.InitialLayout = TextureLayout::TRANSFER_DST;
		textureCreateInfo.MipLevels = 1;

		textureComponent.Texture = Texture(textureCreateInfo);
	}
	
	textures.Insert(loadInfo->Component, textureComponent);
}
