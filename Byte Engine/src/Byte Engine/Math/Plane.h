#pragma once

#include "Vector3.h"

struct Plane
{
	Plane() = default;
	Plane(const Vector3& _A, const Vector3& _B, const Vector3& _C);

	Vector3 Normal;
	float D = 0.0f;
};
