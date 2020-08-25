#pragma once

#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Game/Tasks.h"
#include "ByteEngine/Render/MaterialSystem.h"
#include "ByteEngine/Render/TextureSystem.h"

class TestSystem : public System
{
public:
	void SetTexture(TaskInfo taskInfo, uint32 texture)
	{
		auto* textureSystem = taskInfo.GameInstance->GetSystem<TextureSystem>("TextureSystem");
		auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
		materialSystem->SetMaterialTexture(0, GTSL::Id64(), 0, textureSystem->GetTextureView(texture), textureSystem->GetTextureSampler(texture));
	}

	void Initialize(const InitializeInfo& initializeInfo) override {}
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
private:
};
