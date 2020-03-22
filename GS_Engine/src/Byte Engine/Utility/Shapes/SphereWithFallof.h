#pragma once

class Vector3;

struct SphereWithFalloff
{
	float radius = 0;

	//Additional distance from the shapes inner limit to form the outer limit.
	float falloffDistance = 0;

	SphereWithFalloff();
	SphereWithFalloff(const float DistToOuterLimit);
	SphereWithFalloff(const float DistToOuterLimit, const float FalloffExponent);

	float GetLinearIntensityAt(const Vector3& Position);
	float GetExponentialIntensityAt(const Vector3& Position);
};

//float BoundingSpherewithFallout::GetLinearIntensityAt(const Vector3& Position)
//{
//	return BEM::MapToRangeClamped(BEM::VectorLengthSquared(Transform.Location, Position), 0, DistToOuterLimit * DistToOuterLimit, 0, 1);
//}
