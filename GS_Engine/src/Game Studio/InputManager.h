#pragma once

#include "Core.h"

#include "Object.h"

#include "KeyPressedEvent.h"
#include "MouseMovedEvent.h"

#include "Math\Vector2.h"

GS_CLASS InputManager : public Object
{
public:
	InputManager();
	~InputManager();

	//Event Ids
	uint16 KeyPressedEventId;
	uint16 MouseMovedEventId;

	//Mouse vars
	Vector2 MousePos;

	Vector2 MouseOffset;

	void KeyPressed(KeyboardKeys PressedKey);
	void MouseMoved(const Vector2 & Pos);

};

