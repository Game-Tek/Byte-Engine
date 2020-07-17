#pragma once

#include <ByteEngine.h>

#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Game/GameInstance.h"

class Game final : public GameApplication
{
	GameInstance* sandboxGameInstance{ nullptr };
	GameInstance::WorldReference menuWorld;
	GameInstance::WorldReference gameWorld;
public:
	Game() : GameApplication("Sandbox")
	{
	}

	void Initialize() override;

	void OnUpdate(const OnUpdateInfo& onUpdate) override;

	void Shutdown() override;
	
	~Game();

	const char* GetApplicationName() override { return "Sandbox"; }
};

inline BE::Application* BE::CreateApplication(GTSL::AllocatorReference* allocatorReference)
{
	return GTSL::New<Game>(*allocatorReference);
}

inline void BE::DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference)
{
	Delete(static_cast<Game*>(application), *allocatorReference);
}