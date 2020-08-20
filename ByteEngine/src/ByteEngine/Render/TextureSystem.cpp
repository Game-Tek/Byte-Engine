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
	auto* renderSystem;

	
}
