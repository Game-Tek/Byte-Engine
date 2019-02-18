#pragma once

#include "Core.h"

#include "WorldObject.h"

GS_CLASS Camera : public WorldObject
{
public:
	Camera();
	~Camera();

protected:
	float FOV;
	float FocusDistance;

	float Aperture;

	uint16 WhiteBalance;
	uint16 ISO;
};