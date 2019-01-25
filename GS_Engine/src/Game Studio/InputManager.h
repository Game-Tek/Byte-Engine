#pragma once

#include "Core.h"

#include "Object.h"

#include "KeyPressedEvent.h"
#include "MouseMovedEvent.h"

#include "Vector2.h"

GS_CLASS InputManager : public Object
{
public:
	InputManager();
	~InputManager();

	//Event Ids
	static uint16 KeyPressedEventId;
	static uint16 MouseMovedEventId;

	//Mouse vars
	static Vector2 MousePos;

	static Vector2 MouseOffset;

	static void KeyPressed(Key PressedKey);
	static void MouseMoved(const Vector2 & Pos);

};

