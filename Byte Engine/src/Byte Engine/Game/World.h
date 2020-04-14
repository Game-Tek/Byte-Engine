#pragma once

#include "Object.h"

#include <GTSL/Id.h>
#include "TypeManager.h"
#include <unordered_map>
#include <GTSL/TimePoint.h>

class World : public Object
{
	GTSL::TimePoint levelRunningTime;
	GTSL::TimePoint levelAdjustedRunningTime;
	float worldTimeMultiplier = 1;

	std::unordered_map<GTSL::Id16, TypeManager*> types;
public:
	World();
	virtual ~World();

	[[nodiscard]] const char* GetName() const override { return "World"; }
	
	template<class T>
	void AddTypeManager(const GTSL::Id16& name) { types.insert({ name, new T() }); }
	
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
	[[nodiscard]] GTSL::TimePoint GetWorldRunningTime() const { return levelRunningTime; }
	[[nodiscard]] GTSL::TimePoint GetWorldAdjustedRunningTime() const { return levelAdjustedRunningTime; }
	[[nodiscard]] GTSL::TimePoint GetWorldDeltaTime() const;
};
