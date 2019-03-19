#pragma once

#include "Core.h"

#include "Cone.h"

GS_STRUCT ConeWithFalloff : public Cone
{
	ConeWithFalloff() = default;
	ConeWithFalloff(const float Radius, const float Length);
	ConeWithFalloff(const float Radius, const float Length, const float ExtraRadius);

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