#pragma once

#include "Core.h"

struct MouseState
{
	bool IsRightButtonPressed = false;
	bool IsLeftButtonPressed = false;
	bool IsMouseWheelPressed = false;

	float MouseWheelMove = 0.0f;

	/**
	 * \brief Mouse position in normalized screen coordinates.\n
	 * (-1,  1) --- (1,  1)\n
	 *	  |            |   \n
	 *	  |            |   \n
	 *    |            |   \n
	 * (-1, -1) --- (1, -1)\n
	 */
	Vector2 MousePosition;
};
