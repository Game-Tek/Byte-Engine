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

	void Init() override;

	void OnNormalUpdate() override;

	void OnBackgroundUpdate() override
	{
	}

	~Game();

	[[nodiscard]] const char* GetName() const override { return "Game"; }
	const char* GetApplicationName() override { return "TEST - TEST - TEST - TEST"; }
};

BE::Application* BE::CreateApplication(GTSL::AllocatorReference* allocatorReference)
{
	return GTSL::New<Game>(allocatorReference);
}

void BE::DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference)
{
	Delete(application, allocatorReference);
}