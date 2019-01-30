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

	FVector<Object *> EntityList;

	template <class O>
	void SpawnObject(const O & Object)
	{
		EntityList.push_back(new O(Object));
	}
};