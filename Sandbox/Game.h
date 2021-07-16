#pragma once

#include <ByteEngine.h>


#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Game/CameraSystem.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Render/RenderOrchestrator.h"
#include "ByteEngine/Render/StaticMeshRenderGroup.h"
#include "ByteEngine/Sound/AudioSystem.h"

class Game final : public GameApplication
{
	GameInstance* sandboxGameInstance{ nullptr };
	GameInstance::WorldReference menuWorld;
	GameInstance::WorldReference gameWorld;

	GTSL::Vector2 posDelta;
	GTSL::Vector3 moveDir;
	float32 fov = 45.0f;
	
	CameraSystem::CameraHandle camera;
	StaticMeshHandle hydrant;
	StaticMeshHandle tv;
	MaterialInstanceHandle textMaterial;
	MaterialInstanceHandle tvMat;
	MaterialInstanceHandle buttonMaterial;
	AudioEmitterHandle audioEmitter;
	AudioListenerHandle audioListener;

	bool shouldFire = false;
	MaterialInstanceHandle hydrantMaterialInstance;
	MaterialInstanceHandle tvMaterialInstance;
	StaticMeshHandle plane;
	MaterialInstanceHandle plainMaterialInstance;

	void leftClick(InputManager::ActionInputEvent data);
	void moveLeft(InputManager::ActionInputEvent data);
	void moveForward(InputManager::ActionInputEvent data);
	void moveBackwards(InputManager::ActionInputEvent data);
	void moveRight(InputManager::ActionInputEvent data);
	void zoom(InputManager::LinearInputEvent data);
	void moveCamera(InputManager::Vector2DInputEvent data);
	void view(InputManager::Vector2DInputEvent data);
public:
	Game(GTSL::ShortString<128> name) : GameApplication(name)
	{
	}

	bool Initialize() override;
	void PostInitialize() override;

	void OnUpdate(const OnUpdateInfo& onUpdate) override;

	void Shutdown() override;

	void move(InputManager::Vector2DInputEvent data);
	
	~Game();

	GTSL::ShortString<128> GetApplicationName() override { return u8"Sandbox"; }
};

inline int CreateApplication()
{
	Game applicationInstance(u8"Sandbox");
	const auto res = Start(&applicationInstance);
	return res;
}