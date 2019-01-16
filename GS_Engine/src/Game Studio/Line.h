#pragma once

#include "Core.h"

#include "Vector3.h"

#include "GSM.hpp"

GS_CLASS Line3
{
public:
	Vector3 Start;
	Vector3 End;

	inline float Length() const
	{
		return Segment().Length();
	}

	inline float LengthSquared() const
	{
		return Segment().LengthSquared();
	}

private:
	inline Vector3 Segment() const
	{
		return End - Start;
	}
};