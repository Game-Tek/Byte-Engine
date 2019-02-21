#pragma once

#include "Core.h"
#include "Camera.h"
#include "FVector.hpp"

GS_CLASS Scene
{
public:
	Scene() = default;
	virtual ~Scene() = default;

	void AddWorldObject(WorldObject * Object);
	void RemoveWorldObject(WorldObject * Object);

	void SetCamera(Camera * NewCamera) { ActiveCamera = NewCamera; }
	FVector<WorldObject *> ObjectList;
protected:
	Camera * ActiveCamera = nullptr;
};

