#pragma once

#include "Object.h"

#include "Containers/Id.h"
#include "TypeManager.h"
#include <unordered_map>
#include "Containers/TimePoint.h"

class World : public Object
{
	TimePoint levelRunningTime;
	TimePoint levelAdjustedRunningTime;
	float worldTimeMultiplier = 1;

	std::unordered_map<Id16, TypeManager*> types;
public:
	World();
	virtual ~World();

	[[nodiscard]] const char* GetName() const override { return "World"; }
	
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


	void SetWorldTimeMultiplier(const float multiplier) { worldTimeMultiplier = multiplier; }

	static double GetRealRunningTime();
	[[nodiscard]] TimePoint GetWorldRunningTime() const { return levelRunningTime; }
	[[nodiscard]] TimePoint GetWorldAdjustedRunningTime() const { return levelAdjustedRunningTime; }
	[[nodiscard]] TimePoint GetWorldDeltaTime() const;
};
