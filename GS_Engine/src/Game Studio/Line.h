#pragma once

#include "Core.h"

#include "Math/GSM.hpp"
#include "Math/Vector3.h"

GS_CLASS Line3
{
public:
	Vector3 Start;
	Vector3 End;

	inline float Length() const
	{
		return GSM::VectorLength(Segment());
	}

	inline float LengthSquared() const
	{
		return GSM::VectorLengthSquared(Segment());
	}

private:
	inline Vector3 Segment() const
	{
		return End - Start;
	}
};