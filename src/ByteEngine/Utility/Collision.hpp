#pragma once

#include <GTSL/Range.hpp>

using Vector3MultiRange = GTSL::MultiRange<const float, const float, const float>;

template<typename T>
auto Lookup(GTSL::Range<const T*> range, const float32 t) {
	auto index = (uint32)t;
	auto index1 = GTSL::Math::Limit(index, range.ElementCount());
	return Lerp(range[index], range[index1], t - (float32)index);
}

void AABBvAABB(Vector3MultiRange posA, Vector3MultiRange posB, Vector3MultiRange hWidthA, Vector3MultiRange hWidthB) {
	return Abs(posB - posA) <= (hWidthA + hWidthB);
}

void RemakeAABB(const GTSL::Vector3 localMax, const GTSL::Matrix4& orientation, GTSL::Vector3* newAABB) {
	*newAABB = orientation * localMax;
}

void RemakeAABB(const GTSL::Vector3 localMax, const GTSL::Quaternion& orientation, GTSL::Vector3* newAABB) {
	*newAABB = orientation * localMax;
}