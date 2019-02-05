#pragma once

#include "Core.h"

#include "Object.h"

#include "WorldObject.h"

#include "FVector.hpp"

GS_CLASS World
{
public:
	World();
	~World();

	FVector<WorldObject *> EntityList;

	template <class O>
	void SpawnObject(const O & Object, const Vector3 & Position)
	{
		WorldObject * ptr = new O(Object);
		ptr->SetPosition(Position);
		EntityList.push_back(new O(Object));
	}
};