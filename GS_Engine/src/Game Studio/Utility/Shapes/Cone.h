#pragma once

#include "Core.h"

struct GS_API Cone
{
	Cone() = default;

	Cone(const float Radius, const float Length);

	~Cone() = default;

	//Returns the value of Radius.
	float GetRadius() const { return Radius; }
	//Returns the value of Length.
	float GetLength() const { return Length; }

	float GetInnerAngle() const;

	//Sets Radius as NewRadius.
	void SetRadius(float NewRadius);
	//Sets Length as NewLength.
	void SetLength(float NewLength);

protected:
	//Specifies the radius of the cone.
	float Radius = 100.0f;

	//Specifies the length of the cone.
	float Length = 500.0f;
};

INLINE void Cone::SetRadius(const float NewRadius)
{
	Radius = NewRadius;

	return;
}

INLINE void Cone::SetLength(const float NewLength)
{
	Length = NewLength;

	return;
}
