#pragma once

#include "Object.h"

#include "Containers/Id.h"
#include "TypeManager.h"
#include <unordered_map>

class World : public Object
{
	double levelRunningTime = 0;
	double levelAdjustedRunningTime = 0;
	float worldTimeMultiplier = 1;

	std::unordered_map<Id16, TypeManager*> types;
public:
	World();
	virtual ~World();

	template<class T>
	void AddTypeManager(const Id16& name) { types.insert({ name, new T() }); }
	
	virtual void OnUpdate();

	virtual void Pause();

	struct CreateWorldObject
	{};
	virtual void CreateWorldObject(const CreateWorldObject& createWorldObject);

	struct DestroyWorldObject
	{};
	virtual void DestroyWorldObject(const DestroyWorldObject& destroyWorldObject);

	[[nodiscard]] const char* GetName() const override { return "World"; }

	void SetWorldTimeMultiplier(const float multiplier) { worldTimeMultiplier = multiplier; }

	static double GetRealRunningTime();
	[[nodiscard]] double GetWorldRunningTime() const { return levelRunningTime; }
	[[nodiscard]] double GetWorldAdjustedRunningTime() const { return levelAdjustedRunningTime; }
	[[nodiscard]] float GetWorldDeltaTime() const;
};
