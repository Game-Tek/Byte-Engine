#pragma once

#include "Byte Engine/Core.h"

struct Cone
{
	Cone() = default;

	Cone(const float Radius, const float Length);

	~Cone() = default;

	//Returns the value of Radius.
	[[nodiscard]] float GetRadius() const { return Radius; }
	//Returns the value of Length.
	[[nodiscard]] float GetLength() const { return Length; }

	[[nodiscard]] float GetInnerAngle() const;

	//Sets Radius as NewRadius.
	void SetRadius(float NewRadius)
	{
		Radius = NewRadius;
	}
	//Sets Length as NewLength.
	void SetLength(float NewLength)
	{
		Length = NewLength;
	}

protected:
	//Specifies the radius of the cone.
	float Radius = 100.0f;

	//Specifies the length of the cone.
	float Length = 500.0f;
};