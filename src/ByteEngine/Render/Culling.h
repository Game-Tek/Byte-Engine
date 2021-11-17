#pragma once

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Vectors.h>

#include "ByteEngine/Core.h"
#include <GTSL/SIMD/SIMD.hpp>

struct AABB;
struct vec4;

GTSL::Vector3 ttt(const float32 near, const float32 far, const float32 fov, GTSL::Extent2D size, const GTSL::Matrix4& a) {
	GTSL::Vector3 farPoint(0, 0, far);

	auto xSize = far * GTSL::Math::Tangent(fov / 2);

	a * farPoint;

	GTSL::Vector3 halfSize();

	return halfSize;
}

float32 projectSphere(const GTSL::Vector3 cameraPosition, const GTSL::Vector3 spherePosition, const float32 radius)
{
	//return GTSL::Math::Tangent(radius / GTSL::Math::Length(cameraPosition, spherePosition));
	return GTSL::Math::Tangent((radius * radius) / GTSL::Math::LengthSquared(spherePosition, cameraPosition));
}

float32 test(const GTSL::Vector3 cameraPosition, const GTSL::Vector3 spherePosition, const float32 radius, const float32 fov)
{
	auto size = projectSphere(cameraPosition, spherePosition, radius);
	return GTSL::Math::MapToRangeZeroToOne(fov, 180.f, 1.0f);
}

uint8 SelectLOD(const float32 percentage, const uint8 minLOD, const uint8 maxLOD) {
	return static_cast<uint8>(GTSL::Math::MapToRange(percentage, 0, 1, minLOD, maxLOD));
}