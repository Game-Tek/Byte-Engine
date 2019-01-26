#include "InputManager.h"

#include "EventDispatcher.h"

#include "Logger.h"

uint16 InputManager::KeyPressedEventId = 0;
uint16 InputManager::MouseMovedEventId = 0;

Vector2 InputManager::MousePos;
Vector2 InputManager::MouseOffset;

InputManager::InputManager()
{
	KeyPressedEventId = EventDispatcher::CreateEvent();
	MouseMovedEventId = EventDispatcher::CreateEvent();
}

InputManager::~InputManager()
{
}

void InputManager::KeyPressed(Key PressedKey)
{
	EventDispatcher::Notify<KeyPressedEvent>(KeyPressedEventId, KeyPressedEvent(PressedKey));

	GS_LOG_MESSAGE("Key Pressed")
}

void InputManager::MouseMoved(const Vector2 & Pos)
{
	//Update MouseOffset.
	MouseOffset = Pos - MousePos;

	//If the mouse's position dosn't equal last frame's position update. This is to avoid unnecesary event posts.
	if (MousePos != Pos)
	{
		EventDispatcher::Notify<MouseMovedEvent>(MouseMovedEventId, MouseMovedEvent(MouseOffset));

		GS_LOG_MESSAGE("Mouse Moved: %f, %f", Pos.X, Pos.Y)
	}

	//Set mouse position as the current position.
	MousePos = Pos;
}
