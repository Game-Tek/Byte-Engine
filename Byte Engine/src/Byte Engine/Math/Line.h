#pragma once

#include "Core.h"

#include "BEM.hpp"
#include "Vector3.h"

class Line3
{
public:
	Vector3 Start;
	Vector3 End;

	[[nodiscard]] float Length() const
	{
		return BEM::Length(Segment());
	}

	[[nodiscard]] float LengthSquared() const
	{
		return BEM::LengthSquared(Segment());
	}

private:
	[[nodiscard]] Vector3 Segment() const
	{
		return End - Start;
	}
};
