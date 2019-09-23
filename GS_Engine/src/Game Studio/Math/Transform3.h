#pragma once

#include "Core.h"

#include "Vector3.h"
#include "Quaternion.h"

//Used to specify a transform in 3D space with floating point precision.
struct GS_API Transform3
{
	Vector3 Position;
	Quaternion Rotation;
	Vector3 Scale;

	Transform3() = default;

	Transform3(const Vector3 & _Pos, const Quaternion & _Quat, const Vector3 & _Scale) : Position(_Pos), Rotation(_Quat), Scale(_Scale)
	{
	}

	Transform3(const Transform3& Other) = default;
};