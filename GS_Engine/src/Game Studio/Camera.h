#pragma once

#include "Core.h"

#include "WorldObject.h"

GS_CLASS Camera : public WorldObject
{
public:
	Camera() = default;
	explicit Camera(const float FOV);
	~Camera() = default;

	float GetFOV() const { return FOV; }
	float GetFocusDistance() const { return FocusDistance; }

	float GetAperture() const { return Aperture; }

	void SetFOV(const float NewFOV) { FOV = NewFOV; }
	void SetFocusDistance(const float NewFocusDistance) { FocusDistance = NewFocusDistance; }
	void SetFocusDistance(const Vector3 & Object);

	void SetAperture(const float NewAperture) { Aperture = NewAperture; }

protected:
	float FOV = 45.0f;
	float FocusDistance = 0.0f;

	float Aperture = 2.8f;

	uint16 WhiteBalance = 4000;
	uint16 ISO = 1800;
};