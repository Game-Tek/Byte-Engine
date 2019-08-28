#include "InputManager.h"

#include "RAPI/Window.h"

void InputManager::OnUpdate()
{
	auto& MouseState = ActiveWindow->GetMouseState();

	MouseOffset = MouseState.MousePosition - MousePosition;
	MousePosition = MouseState.MousePosition;

	Mouse = MouseState;

	ScrollWheelDelta = MouseState.MouseWheelMove - ScrollWheelMovement;
	ScrollWheelMovement = MouseState.MouseWheelMove;

	Keys = ActiveWindow->GetKeyboardKeys();
	JoystickStates = ActiveWindow->GetJoystickStates();
}
