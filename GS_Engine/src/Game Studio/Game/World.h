#pragma once

#include "Object.h"

#include "Containers/FVector.hpp"
#include "WorldObject.h"
#include "Render/Renderer.h"

class World : public Object
{
	FVector<WorldObject*> WorldObjects;

	Renderer WorldScene;

	double levelRunningTime = 0;
	double levelAdjustedRunningTime = 0;
	float worldTimeMultiplier = 1;

public:
	World();
	virtual ~World();

	void OnUpdate() override;

	template <class T>
	T* CreateWorldObject()
	{
		WorldObject* Obj = new T();

		Obj->SetID(static_cast<WorldObjectID>(WorldObjects.getLength()));

		WorldObjects.push_back(Obj);

		return static_cast<T*>(Obj);
	}

	void DestroyWorldObject(WorldObject* _Object)
	{
		delete WorldObjects[_Object->GetID()];
	}

	[[nodiscard]] const char* GetName() const override { return "World"; }

	[[nodiscard]] Renderer& GetScene() { return WorldScene; }

	void SetWorldTimeMultiplier(const float multiplier) { worldTimeMultiplier = multiplier; }

	double GetWorldRunningTime() const { return levelRunningTime; }
	double GetWorldAdjustedRunningTime() const { return levelAdjustedRunningTime; }
	static double GetRealRunningTime();
	float GetWorldDeltaTime() const;
};
