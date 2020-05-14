#include <ByteEngine.h>

#include "SandboxWorld.h"
#include "Byte Engine/Application/Templates/GameApplication.h"
#include "Byte Engine/Game/GameInstance.h"

class Game final : public GameApplication
{
	GameInstance* sandboxGameInstance{ nullptr };
	GameInstance::WorldReference menuWorld;
	GameInstance::WorldReference gameWorld;
public:
	Game() : GameApplication("Sandbox")
	{
	}

	void Init() override
	{
		GameApplication::Init();

		//GameInstance::CreateNewWorldInfo create_new_world_info;
		//create_new_world_info.Application = this;
		//menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

		BE_LOG_SUCCESS("Inited Game!")
		
		//show loading screen
		//load menu
		//show menu
		//start game
	}
	
	void OnNormalUpdate() override
	{
		GameApplication::OnNormalUpdate();
		BE_LOG_MESSAGE("Hello!")
	}

	void OnBackgroundUpdate() override
	{
	}

	~Game()
	{
	}

	[[nodiscard]] const char* GetName() const override { return "Game"; }
	const char* GetApplicationName() override { return "Game"; }
};

BE::Application* BE::CreateApplication(GTSL::AllocatorReference* allocatorReference)
{
	void* gameAlloc{ nullptr };
	uint64 allocSize{ 0 };
	allocatorReference->Allocate(sizeof(Game), alignof(Game), &gameAlloc, &allocSize);
	return ::new(gameAlloc) Game();
}

void BE::DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference)
{
	static_cast<Game*>(application)->~Game();
	allocatorReference->Deallocate(sizeof(Game), alignof(Game), application);
}