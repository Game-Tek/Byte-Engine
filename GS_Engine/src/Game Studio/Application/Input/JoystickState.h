#pragma once
#include "Core.h"
#include "Math/Vector2.h"

struct GS_API JoystickState
{
	Vector2 LeftJoystickPosition;
	bool IsLeftStickPressed = false;
	Vector2 RightJoystickPosition;
	bool IsRightStickPressed = false;

	bool IsUpFaceButtonPressed = false;
	bool IsRightFaceButtonPressed = false;
	bool IsBottomFaceButtonPressed = false;
	bool IsLeftFaceButtonPressed = false;

	float RightTriggerDepth = 0.0f;
	bool IsRightBumperPressed = false;
	float LeftTriggerDepth = 0.f;
	bool IsLeftBumperPressed = false;

	bool IsUpDPadButtonPressed = false;
	bool IsRightDPadButtonPressed = false;
	bool IsDownDPadButtonPressed = false;
	bool IsLeftDPadButtonPressed = false;

	bool IsRightMenuButtonPressed = false;
	bool IsLeftMenuButtonPressed = false;
};
