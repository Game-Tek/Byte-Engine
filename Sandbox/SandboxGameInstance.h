#pragma once

#include <ByteEngine/Game/ApplicationManager.h>

class SandboxGameInstance final : public ApplicationManager
{
public:
	////void OnUpdate(BE::Application* application) override
	//{
	//	ApplicationManager::OnUpdate(application);
	//
	//	//GTSL::HashMap<float32> map(2, GetTransientAllocator());
	//	//
	//	//map.Emplace(GetTransientAllocator(), 25, 25.32f);
	//	//
	//	//static uint32 i = 0;
	//	//
	//	//GTSL::ForEach(map, [&](float32& number) { BE_LOG_MESSAGE(number, ' ', BE::Application::Get()->GetApplicationTicks()); });
	//	//
	//	//map.Free(GetTransientAllocator());
	//}
};