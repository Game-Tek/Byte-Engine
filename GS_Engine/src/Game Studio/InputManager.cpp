#include "InputManager.h"

#include "Logger.h"
#include "Application.h"

InputManager::InputManager()
{
}

InputManager::~InputManager()
{
}

void InputManager::KeyPressed(KeyboardKeys PressedKey)
{
	GS_LOG_MESSAGE("Key Pressed")
}

void InputManager::MouseMoved(const Vector2 & Pos)
{
	//Update MouseOffset.
	MouseOffset = Pos - MousePos;

	//If the mouse's position dosn't equal last frame's position update don't post an event. This is to avoid unnecesary event posts.
	if (MousePos != Pos)
	{
		//GS_LOG_MESSAGE("Mouse Moved: %f, %f", Pos.X, Pos.Y)
	}

	//Set mouse position as the current position.
	MousePos = Pos;
}
