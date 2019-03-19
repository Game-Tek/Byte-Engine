#pragma once

#include "Core.h"

#include "Cone.h"

GS_STRUCT ConeWithFallof : public Cone
{
	ConeWithFallof() = default;
	ConeWithFallof(const float Radius, const float Length);
	ConeWithFallof(const float Radius, const float Length, const float ExtraRadius);

	~ConeWithFallof() = default;

	//Returns the value of ExtraRadius.
	float GetExtraRadius() const { return ExtraRadius; }

	//Sets Extra Radius as NewExtraRadius.
	void SetExtraRadius(const float NewExtraRadius);

	float GetOuterConeInnerRadius() const;

protected:
	//Determines the extra radius on top of the original radius to determine the outer radius.
	float ExtraRadius = 10.0f;
};

ConeWithFallof::ConeWithFallof(const float Radius, const float Length) : Cone(Radius, Length)
{
}

ConeWithFallof::ConeWithFallof(const float Radius, const float Length, const float ExtraRadius) : Cone(Radius, Length), ExtraRadius(ExtraRadius)
{
}

INLINE void ConeWithFallof::SetExtraRadius(const float NewExtraRadius)
{
	ExtraRadius = NewExtraRadius;

	return;
}