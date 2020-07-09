#pragma once

#include "ByteEngine/Application/Application.h"

#include <GTSL/Window.h>
#include <GTSL/GamepadQuery.h>

class GameApplication : public BE::Application
{
public:
	GameApplication(const char* name) : Application(BE::ApplicationCreateInfo{ name })
	{
	}

	void Initialize() override;
	void OnUpdate(const OnUpdateInfo& updateInfo) override;
	void Shutdown() override;

protected:
	GTSL::Window window;

	GTSL::GamepadQuery gamepad;
	
	void SetupInputSources();
	void RegisterMouse();
	void RegisterKeyboard();
	void RegisterControllers();
};
