#pragma once

#include "Application.h"

#define WIN32_LEAN_AND_MEAN
#include <Windows.h>

#include "Xinput.h"

class WindowsApplication final : public Application
{
	HINSTANCE instance = nullptr;

	uint8 connectedControllers = 0;

	XINPUT_STATE input_states[XUSER_MAX_COUNT];

	static constexpr GamepadButtonState intToGamepadButtonState(const int a) {	return static_cast<GamepadButtonState>(!a); }
public:
	explicit WindowsApplication(const ApplicationCreateInfo& applicationCreateInfo);

	void Update() override;

	void Close() override;

	void CheckForNewControllers() override;
	
	[[nodiscard]] HINSTANCE GetInstance() const { return instance; }
};
