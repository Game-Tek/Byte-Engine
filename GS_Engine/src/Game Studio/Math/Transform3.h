#pragma once

#include "Core.h"

#include "Vector3.h"
#include "Quaternion.h"
#include "Rotator.h"

//Used to specify a transform in 3D space with floating point precision.
struct Transform3
{
	Vector3 Position;
	Quaternion Rotation;
	Vector3 Scale;

	Transform3() = default;

	Transform3(const Vector3& _Pos, const Quaternion& rotator, const Vector3& _Scale) : Position(_Pos),
	                                                                                    Rotation(rotator), Scale(_Scale)
	{
	}

	Transform3(const Transform3& Other) = default;
};
