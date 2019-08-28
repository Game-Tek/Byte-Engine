#pragma once

#include "Core.h"

#include "Extent.h"
#include "Containers/FString.h"

#include "Math/Vector2.h"

#include "../InputEnums.h"
#include "MouseState.h"
#include "Containers/Array.hpp"
#include "JoystickState.h"

enum class WindowFit : uint8
{
	NORMAL, MAXIMIZED, FULLSCREEN
};

GS_STRUCT WindowCreateInfo
{
	Extent2D Extent;
	FString Name;
	WindowFit WindowType = WindowFit::NORMAL;
};

GS_CLASS Window
{
protected:
	Extent2D Extent;
	WindowFit Fit;

	MouseState WindowMouseState;

	bool ShouldClose = false;

	Array<bool, MAX_KEYBOARD_KEYS> Keys;

	uint8 JoystickCount = 0;
	Array<JoystickState, 4> JoystickStates;

public:
	Window(Extent2D _Extent, WindowFit _Fit) : Extent(_Extent), Fit(_Fit)
	{
	}

	virtual ~Window() {};

	static Window* CreateGSWindow(const WindowCreateInfo& _WCI);

	virtual void Update() = 0;

	virtual void SetWindowFit(WindowFit _Fit) = 0;
	virtual void MinimizeWindow() = 0;
	virtual void NotifyWindow() = 0;

	virtual void SetWindowTitle(const char* _Title) = 0;

	[[nodiscard]] const Extent2D& GetWindowExtent() const { return Extent; }
	[[nodiscard]] const MouseState& GetMouseState() const { return WindowMouseState; }
	[[nodiscard]] uint8 GetJoystickCount() const { return JoystickCount; }
	[[nodiscard]] const Array<JoystickState, 4> & GetJoystickStates() const { return JoystickStates; }
	[[nodiscard]] const Array<bool, MAX_KEYBOARD_KEYS>& GetKeyboardKeys() const { return Keys; }

	INLINE bool GetShouldClose() const { return ShouldClose; }
	INLINE float GetAspectRatio() const { return SCAST(float, Extent.Width) / SCAST(float, Extent.Height); }
};