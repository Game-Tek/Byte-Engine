#pragma once

#include "Core.h"

#include "GSM.hpp"
#include "Vector3.h"

class GS_API Line3
{
public:
	Vector3 Start;
	Vector3 End;

	[[nodiscard]] float Length() const
	{
		return GSM::Length(Segment());
	}

	[[nodiscard]] float LengthSquared() const
	{
		return GSM::LengthSquared(Segment());
	}

private:
	[[nodiscard]] Vector3 Segment() const
	{
		return End - Start;
	}
};
