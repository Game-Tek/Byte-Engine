#pragma once

#include "Core.h"
#include "Object.h"

#include "Containers/FVector.hpp"
#include "WorldObject.h"
#include "Render/Scene.h"

class GS_API World : public Object
{
	FVector<WorldObject*> WorldObjects;

	Scene WorldScene;
public:
	World() = default;
	virtual ~World();

	void OnUpdate() override
	{
		for (WorldObject* WorldObject : WorldObjects)
		{
			WorldObject->OnUpdate();
		}
	}

	template<class T>
	WorldObject* CreateWorldObject(const Vector3& _Pos)
	{
		WorldObject* Obj = new T();
		WorldObjects.push_back(Obj);

		Obj->SetID(WorldObjects.length());
		Obj->SetPosition(_Pos);

		return Obj;
	}

	void DestroyWorldObject(WorldObject* _Object)
	{
		delete WorldObjects[_Object->GetID()];
	}

	[[nodiscard]] const char* GetName() const override { return "World"; }

	[[nodiscard]] const Scene& GetScene() const { return WorldScene; }

};