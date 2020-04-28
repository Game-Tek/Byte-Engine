#pragma once

#include <GTSL/Id.h>
#include "TypeManager.h"
#include <unordered_map>
#include <GTSL/Time.h>
#include "Byte Engine/Object.h"

class World : public Object
{
	float worldTimeMultiplier = 1;

	std::unordered_map<GTSL::Id64::HashType, TypeManager*> types;
public:
	World();
	virtual ~World();

	[[nodiscard]] const char* GetName() const override { return "World"; }
	
	template<class T>
	void AddTypeManager(const GTSL::Id64& name) { types.insert({ name, new T() }); }
	
	virtual void OnUpdate();

	virtual void Pause();

	struct CreateWorldObject
	{};
	virtual void CreateWorldObject(const CreateWorldObject& createWorldObject);

	struct DestroyWorldObject
	{};
	virtual void DestroyWorldObject(const DestroyWorldObject& destroyWorldObject);


	void SetWorldTimeMultiplier(const float multiplier) { worldTimeMultiplier = multiplier; }
};
