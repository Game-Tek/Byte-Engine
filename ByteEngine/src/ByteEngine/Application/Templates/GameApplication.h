#pragma once

#include "ByteEngine/Application/Application.h"

#include <GTSL/Window.h>
#include <GTSL/Gamepad.h>

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Game/GameInstance.h"

class GameApplication : public BE::Application
{
public:
	GameApplication(const char* name) : Application(BE::ApplicationCreateInfo{ name })
	{
	}

	~GameApplication() = default;
	
	bool Initialize() override;
	void PostInitialize() override;
	void OnUpdate(const OnUpdateInfo& updateInfo) override;
	void Shutdown() override;

	// EVENTS
	static EventHandle<bool> GetOnFocusGainEventHandle() { return EventHandle<bool>("OnFocusGain"); }
	static EventHandle<bool> GetOnFocusLossEventHandle() { return EventHandle<bool>("OnFocusLoss"); }

protected:
	GTSL::Window window;
	GTSL::Extent2D oldSize;

	GTSL::Gamepad gamepad;
	InputDeviceHandle controller;
	InputDeviceHandle keyboard;
	InputDeviceHandle mouse;

	void SetupInputSources();
	void RegisterMouse();
	void RegisterKeyboard();
	void RegisterControllers();

	void onWindowResize(const GTSL::Extent2D& extent);

	void keyboardEvent(const GTSL::Window::KeyboardKeys key, const bool state, bool isFirstkeyOfType);
	void windowUpdateFunction(void* userData, GTSL::Window::WindowEvents event, void* eventData);
};
