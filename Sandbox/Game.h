#pragma once

#include <ByteEngine.h>


#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Game/GameInstance.h"

class Game final : public GameApplication
{
	GameInstance* sandboxGameInstance{ nullptr };
	GameInstance::WorldReference menuWorld;
	GameInstance::WorldReference gameWorld;

	GTSL::Vector2 posDelta;
	
	uint32 camera;
	void moveLeft(InputManager::ActionInputEvent data);
	void moveRight(InputManager::ActionInputEvent data);
public:
	Game() : GameApplication("Sandbox")
	{
	}

	void Initialize() override;
	void PostInitialize() override;

	void OnUpdate(const OnUpdateInfo& onUpdate) override;

	void Shutdown() override;

	void move(InputManager::Vector2DInputEvent data);
	
	~Game();

	const char* GetApplicationName() override { return "Sandbox"; }
};

inline GTSL::SmartPointer<BE::Application, SystemAllocatorReference> CreateApplication(const SystemAllocatorReference& allocatorReference)
{
	return GTSL::SmartPointer<BE::Application, SystemAllocatorReference>::Create<Game>(allocatorReference);
}