#pragma once

#include "Core.h"

#include "Cone.h"

GS_STRUCT ConeWithFalloff : public Cone
{
	ConeWithFalloff() = default;
	ConeWithFalloff(float Radius, float Length);
	ConeWithFalloff(float Radius, float Length, float ExtraRadius);

	~ConeWithFalloff() = default;

	//Returns the value of ExtraRadius.
	float GetExtraRadius() const { return ExtraRadius; }

	//Sets Extra Radius as NewExtraRadius.
	void SetExtraRadius(const float NewExtraRadius);

	float GetOuterConeInnerRadius() const;

protected:
	//Determines the extra radius on top of the original radius to determine the outer radius.
	float ExtraRadius = 50.0f;
};

INLINE void ConeWithFalloff::SetExtraRadius(const float NewExtraRadius)
{
	ExtraRadius = NewExtraRadius;

	return;
}