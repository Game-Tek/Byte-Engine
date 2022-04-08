#pragma once

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Vectors.hpp>

#include <GTSL/Range.hpp>

#include "ByteEngine/Core.h"
#include <GTSL/SIMD.hpp>

struct vec4;

//GTSL::Vector3 ttt(const float32 near, const float32 far, const float32 fov, GTSL::Extent2D size, const GTSL::Matrix4& a) {
//	GTSL::Vector3 farPoint(0, 0, far);
//
//	auto xSize = far * GTSL::Math::Tangent(fov / 2);
//
//	a * farPoint;
//
//	GTSL::Vector3 halfSize();
//}

inline float32 projectSphere(GTSL::Vector3 cameraPosition, GTSL::Vector3 spherePosition, const float32 radius) {
	return GTSL::Math::Tangent(radius * radius / GTSL::Math::DistanceSquared(spherePosition, cameraPosition));
}

inline void projectSpheres(const GTSL::Vector3 cameraPosition, GTSL::MultiRange<float32, float32, float32, float32> spherePositions, auto& results) {
	using float8x = GTSL::SIMD<float32, 8>;

	float8x cameraX(cameraPosition[0]), cameraY(cameraPosition[1]), cameraZ(cameraPosition[2]);

	for(uint32 i = 0; i < spherePositions.GetLength(); i += 8) {
		float8x sphereX(spherePositions.GetPointer<0>(i)), sphereY(spherePositions.GetPointer<1>(i)), sphereZ(spherePositions.GetPointer<2>(i)), sphereRadiuses(spherePositions.GetPointer<3>(i));

		float8x distanceX = cameraX - sphereX, distanceY = cameraY - sphereY, distanceZ = cameraZ - sphereZ;
		float8x distanceSquared = distanceX * distanceX + distanceY * distanceY + distanceZ * distanceZ;

		auto result = (sphereRadiuses * sphereRadiuses) / distanceSquared;
		//auto result = GTSL::Math::Tangent((sphereRadiuses * sphereRadiuses) / distanceSquared);

		float32 res[8];
		result.CopyTo(res);

		for (auto j = 0; j < 8; ++j) { results.EmplaceBack(res[j]); }
	}
}

inline float32 test(const GTSL::Vector3 cameraPosition, const GTSL::Vector3 spherePosition, const float32 radius, const float32 fov)
{
	auto size = projectSphere(cameraPosition, spherePosition, radius);
	return GTSL::Math::MapToRangeZeroToOne(fov, 180.f, 1.0f);
}

inline uint8 SelectLOD(const float32 percentage, const uint8 minLOD, const uint8 maxLOD) {
	return static_cast<uint8>(GTSL::Math::MapToRange(percentage, 0, 1, minLOD, maxLOD));
}


using AABB2 = GTSL::Vector2;
using AABB = GTSL::Vector3;

inline void ScreenCull(const GTSL::Range<const AABB*> aabbs) {
	GTSL::StaticVector<AABB, 16> front; front.EmplaceBack();

	for(uint32 i = 0; i < aabbs.ElementCount(); ++i) {
		
	}
}