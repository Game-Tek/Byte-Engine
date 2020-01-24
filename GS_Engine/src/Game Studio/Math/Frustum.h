#pragma once

#include "Plane.h"

class Frustum
{
	Plane Planes[6];

	// TOP
	// RIGHT
	// BOTTOM
	// LEFT
	// FRONT
	// BACK
	
public:
	Frustum() = default;
	
	[[nodiscard]] Plane& GetTopPlane() { return Planes[0]; }
	[[nodiscard]] Plane& GetRightPlane() { return Planes[1]; }
	[[nodiscard]] Plane& GetBottomPlane() { return Planes[2]; }
	[[nodiscard]] Plane& GetLeftPlane() { return Planes[3]; }
	[[nodiscard]] Plane& GetFrontPlane() { return Planes[4]; }
	[[nodiscard]] Plane& GetBackPlane() { return Planes[5]; }

	[[nodiscard]] Plane* GetPlanes() { return Planes; }
};
