#pragma once

#include "Core.h"

//Used to specify a rotation with floating point precision.
class GS_API Rotator
{
public:
	//Yaw(Y) component of this rotator.
	float Yaw = 0;

	//Pitch(X) component of this rotator.
	float Pitch = 0;

	//Roll(Z) component of this rotator.
	float Roll = 0;

	Rotator()
	{
	}

	Rotator(float Yaw, float Pitch, float Roll) : Yaw(Yaw), Pitch(Pitch), Roll(Roll)
	{
	}

	Rotator(const Rotator & Other) : Yaw(Other.Yaw), Pitch(Other.Pitch), Roll(Other.Roll)
	{
	}

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