#include <ByteEngine.h>

#include "SandboxWorld.h"
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
	return GTSL::New<Game>(allocatorReference);
}

void BE::DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference)
{
	Delete(application, allocatorReference);
}