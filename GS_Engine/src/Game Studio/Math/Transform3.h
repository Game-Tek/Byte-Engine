#pragma once

#include "Core.h"

#include "Vector3.h"
#include "Rotator.h"

//Used to specify a transform in 3D space with floating point precision.
GS_CLASS Transform3
{
public:
	Transform3()
	{
	}

	Transform3(const Vector3 & Pos, const Rotator & Rot, const Vector3 & Sca) : Position(Pos), Rotation(Rot), Scale(Sca)
	{
	}

	Transform3(const Transform3 & Other) : Position(Other.Position), Rotation(Other.Rotation), Scale(Other.Scale)
	{
	}

	Vector3 Position;
	Rotator Rotation;
	Vector3 Scale;
};