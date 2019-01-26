#include "InputManager.h"

#include "EventDispatcher.h"

#include "Logger.h"

#include <iostream>

#include <stdio.h>

uint16 InputManager::KeyPressedEventId = 0;
uint16 InputManager::MouseMovedEventId = 0;

Vector2 InputManager::MousePos;
Vector2 InputManager::MouseOffset;

InputManager::InputManager()
{
	KeyPressedEventId = EventDispatcher::CreateEvent();
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
	MouseOffset = Pos - MousePos;

	if (MousePos != Pos)
	{
		EventDispatcher::Notify<MouseMovedEvent>(MouseMovedEventId, MouseMovedEvent(MouseOffset));
	}

	MousePos = Pos;

	GS_LOG_MESSAGE("Mouse Moved: %f, %f", Pos.X, Pos.Y)

	//std::cout << Pos.Y << std::endl;
}
