#pragma once

#include "Byte Engine/Core.h"

#include "Cone.h"

struct ConeWithFalloff : public Cone
{
	ConeWithFalloff() = default;
	ConeWithFalloff(float Radius, float Length);
	ConeWithFalloff(float Radius, float Length, float ExtraRadius);

	~ConeWithFalloff() = default;

	//Returns the value of ExtraRadius.
	[[nodiscard]] float GetExtraRadius() const { return ExtraRadius; }

	//Sets Extra Radius as NewExtraRadius.
	void SetExtraRadius(const float NewExtraRadius)
	{
		ExtraRadius = NewExtraRadius;
	}

	[[nodiscard]] float GetOuterConeInnerRadius() const;

protected:
	//Determines the extra radius on top of the original radius to determine the outer radius.
	float ExtraRadius = 50.0f;
};