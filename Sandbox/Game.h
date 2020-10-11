#pragma once

#include <ByteEngine.h>


#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Render/MaterialSystem.h"

class Game final : public GameApplication
{
	GameInstance* sandboxGameInstance{ nullptr };
	GameInstance::WorldReference menuWorld;
	GameInstance::WorldReference gameWorld;

	GTSL::Vector2 posDelta;
	GTSL::Vector3 moveDir;
	float32 fov = 45.0f;
	
	ComponentReference camera;
	MaterialHandle material;
	MaterialHandle textMaterial;
	ComponentReference texture;
	MaterialHandle tvMat;
	void moveLeft(InputManager::ActionInputEvent data);
	void moveForward(InputManager::ActionInputEvent data);
	void moveBackwards(InputManager::ActionInputEvent data);
	void moveRight(InputManager::ActionInputEvent data);
	void zoom(InputManager::LinearInputEvent data);
	void view(InputManager::Vector2DInputEvent data);
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