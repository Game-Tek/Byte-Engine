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
	T* CreateWorldObject()
	{
		WorldObject* Obj = new T();
		WorldObjects.push_back(Obj);

		Obj->SetID(WorldObjects.length());

		return SCAST(T*, Obj);
	}

	template<class T>
	T* CreateWorldObject(const Vector3& _Pos)
	{
		WorldObject* Obj = new T();
		WorldObjects.push_back(Obj);

		Obj->SetID(WorldObjects.length());
		Obj->SetPosition(_Pos);

		return SCAST(T*, Obj);
	}

	void DestroyWorldObject(WorldObject* _Object)
	{
		delete WorldObjects[_Object->GetID()];
	}

	[[nodiscard]] const char* GetName() const override { return "World"; }

	[[nodiscard]] const Scene& GetScene() const { return WorldScene; }

};