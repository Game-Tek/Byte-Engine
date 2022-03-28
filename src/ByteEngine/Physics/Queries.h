#pragma once

#include <utility>
#include <array>
#include <GTSL/Pair.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Core.h"
#include "ByteEngine/Application/AllocatorReferences.h"

class Obj
{
public:
	GTSL::Vector3 GetPosition() { return GTSL::Vector3(); }
	
	GTSL::Vector3 GetSupportPointInDirection(const GTSL::Vector3& direction) {
		return GTSL::Vector3();
	}
};