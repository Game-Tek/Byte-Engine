#pragma once

#include "Core.h"
#include "Object.h"

#include "Containers/FVector.hpp"
#include "WorldObject.h"
#include "Render/Scene.h"

class World : public Object
{
	FVector<WorldObject*> WorldObjects;

	Scene WorldScene;
public:
	World();
	virtual ~World();

	void OnUpdate() override
	{
		for (auto& WorldObject : WorldObjects)
		{
			WorldObject->OnUpdate();
		}

		WorldScene.OnUpdate();
	}

	//template<class T>
	//T* CreateWorldObject()
	//{
	//	WorldObject* Obj = new T();
	//	WorldObjects.push_back(Obj);
	//
	//	Obj->SetID(WorldObjects.length());
	//
	//	return SCAST(T*, Obj);
	//}

	template<class T>
	T* CreateWorldObject(const Vector3& _Pos)
	{
		WorldObject* Obj = new T();
		Obj->SetID(WorldObjects.length());
		Obj->SetPosition(_Pos);

		WorldObjects.push_back(Obj);

		return static_cast<T*>(Obj);
	}

	void DestroyWorldObject(WorldObject* _Object)
	{
		delete WorldObjects[_Object->GetID()];
	}

	[[nodiscard]] const char* GetName() const override { return "World"; }

	[[nodiscard]] Scene& GetScene() { return WorldScene; }

};