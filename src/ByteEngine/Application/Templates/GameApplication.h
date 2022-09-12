#pragma once

#include "ByteEngine/Application/Application.h"

#include <GTSL/Window.h>
#include <GTSL/Gamepad.h>

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Game/ApplicationManager.h"

class GameApplication : public BE::Application
{
public:
	GameApplication(GTSL::ShortString<128> name) : Application(name)
	{
	}

	~GameApplication() = default;
	
	bool Initialize() override;
	void PostInitialize() override;
	void OnUpdate(const OnUpdateInfo& updateInfo) override;
	void Shutdown() override;

protected:
	GTSL::Gamepad gamepad;
	InputDeviceHandle controller;
	InputDeviceHandle keyboard;
	InputDeviceHandle mouse;

	SystemHandle windowSystemHandle;

	uint32 mouseCount = 0;

	void SetupInputSources();
	void RegisterMouse();
	void RegisterKeyboard();
	void RegisterControllers();

	void keyboardEvent(const GTSL::Window::KeyboardKeys key, const bool state, bool isFirstkeyOfType);
};
