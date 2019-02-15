#pragma once

#include "Core.h"

#include "Object.h"

#include "WorldObject.h"

#include "FVector.hpp"

GS_CLASS World : public Object
{
public:
	World();
	~World();

	void OnUpdate() override;

	void SpawnObject(WorldObject * NewObject, const Vector3 & Position);

	const FVector<WorldObject *> & GetEntityList() const { return EntityList; }

protected:
	FVector<WorldObject *> EntityList;

};