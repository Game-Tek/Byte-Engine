#pragma once

#include "Core.h"

#include "Object.h"

#include "Math\Vector2.h"

GS_CLASS InputManager : public Object
{
public:
	InputManager();
	~InputManager();

	//Mouse vars
	Vector2 MousePos;

	Vector2 MouseOffset;

};

