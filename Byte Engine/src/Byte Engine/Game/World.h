#pragma once

#include "Byte Engine/Core.h"

#include <GTSL/Id.h>
#include "TypeManager.h"
#include <unordered_map>

#include "Byte Engine/Object.h"

/**
 * \brief Represents an entity.
 * Acts as a reference to an an entity.
 */
struct Entity
{
	GTSL::Id32 type;
	uint32 index{ 0 };
};

class EntitiesManager
{
	std::unordered_map<uint32, TypeManager*> types;

public:
	void AddType(const GTSL::Ranger<char>& name, TypeManager* typeManager) { types.insert({ GTSL::Id32(name.begin()), typeManager }); }

	[[nodiscard]] TypeManager* GetTypeManager(const GTSL::Id32& id) const noexcept { return types.at(id); }

	[[nodiscard]] TypeManager* GetEntity(const Entity& entity) const noexcept { return types.at(entity.type); }

	auto begin() { return types.begin(); }
	auto end() { return types.end().operator++(); }
};

class World : public Object
{
	float worldTimeMultiplier = 1;

	EntitiesManager entitiesManager;
public:
	World();
	virtual ~World() = default;

	[[nodiscard]] const char* GetName() const override { return "World"; }

	struct InitializeInfo
	{
	};
	virtual void InitializeWorld(const InitializeInfo& initializeInfo);

	struct DestroyInfo
	{
	};
	virtual void DestroyWorld(const DestroyInfo& destroyInfo);
	
	virtual void Pause();

	void SetWorldTimeMultiplier(const float multiplier) { worldTimeMultiplier = multiplier; }
};
