#pragma once

#include "Core.h"

GS_STRUCT MouseState
{
	bool IsRightButtonPressed = false;
	bool IsLeftButtonPressed = false;
	bool IsMouseWheelPressed = false;

	float MouseWheelMove = 0.0f;

	Vector2 MousePosition;
};
