#pragma once

#include "Core.h"

//Used to specify a rotation with floating point precision.
GS_CLASS Rotator
{
public:
	//Yaw(Y) component of this rotator.
	float Yaw;

	//Pitch(X) component of this rotator.
	float Pitch;

	//Roll(Z) component of this rotator.
	float Roll;

	Rotator operator+ (const Rotator & Other)
	{
		return { Yaw + Other.Yaw, Pitch + Other.Pitch, Roll + Other.Roll };
	}

	Rotator operator- (const Rotator & Other)
	{
		return { Yaw - Other.Yaw, Pitch - Other.Pitch, Roll - Other.Roll };
	}

	Rotator operator* (float Other)
	{
		return { Yaw * Other, Pitch * Other, Roll * Other };
	}

	Rotator operator/ (float Other)
	{
		return { Yaw / Other, Pitch / Other, Roll / Other };
	}
};