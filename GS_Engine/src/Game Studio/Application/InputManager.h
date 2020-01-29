#pragma once

#include "Core.h"

#include "Object.h"

#include "Math\Vector2.h"
#include "Containers/Array.hpp"
#include "Input/MouseState.h"
#include "Input/InputEnums.h"
#include "Input/JoystickState.h"
#include "Containers/Id.h"
#include "Containers/FVector.hpp"
#include "Utility/Delegate.h"
#include <map>

class Window;

struct InputAction
{
	Id Name;
	FVector<FString> FiringInputEvents;
};

class GS_API InputManager : public Object
{
	Window* ActiveWindow = nullptr;

	MouseState Mouse;

	Array<bool, MAX_KEYBOARD_KEYS> Keys;

	Array<JoystickState, 4> JoystickStates;

	//std::multimap<Id::HashType, Pair<Action, FVector<Functor<void()>>>> SingleActions;
	//std::multimap<Id::HashType, Pair<Action, FVector<Functor<void()>>>> ContinuousActions;

	//Mouse vars

	//Current mouse position.
	Vector2 MousePosition;

	//Offset from last frame's mouse position.
	Vector2 MouseOffset;

	float ScrollWheelMovement = 0.0f;
	float ScrollWheelDelta = 0.0f;

public:
	InputManager() = default;
	~InputManager() = default;

	void SetActiveWindow(Window* _NewWindow) { ActiveWindow = _NewWindow; }

	[[nodiscard]] MouseState GetMouseState() const { return Mouse; }
	[[nodiscard]] bool GetKeyState(KeyboardKeys _Key) const { return Keys[SCAST(uint8, _Key)]; }
	[[nodiscard]] JoystickState GetJoystickState(uint8 _Joystick) const { return JoystickStates[_Joystick]; }

	Vector2 GetMouseOffset() const { return MouseOffset; }

	void OnUpdate() override;

	[[nodiscard]] const char* GetName() const override { return "Input Manager"; }
};
