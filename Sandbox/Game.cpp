#include "Byte Engine/Application/Templates/GameApplication.h"

class Game final : public GameApplication
{
public:
	Game() : GameApplication("Sandbox")
	{
	}

	void Init() override
	{
		GameApplication::Init();

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
