#pragma once

#include "Core.h"
#include "Camera.h"

GS_CLASS Scene
{
public:
	Scene() = default;
	virtual ~Scene() = default;

	void SetCamera(Camera * NewCamera) { ActiveCamera = NewCamera; }

protected:
	Camera * ActiveCamera = nullptr;
};

