#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Vector.hpp>

#include "TypeManager.h"

#include "ByteEngine/Object.h"

/**
 * \brief Represents an entity.
 * Acts as a reference to an an entity.
 */
struct Entity
{
protected:
	uint16 type{ 0 };
	uint32 index{ 0 };

public:
	Entity(const uint16 typeId, const uint32 entityIndex) noexcept : type(typeId), index(entityIndex)
	{
	}
	
	[[nodiscard]] uint16 GetType() const { return type; }
	[[nodiscard]] uint32 GetIndex() const { return index; }
};

class EntitiesManager
{
	GTSL::Vector<uint64> hashes;
	GTSL::Vector<TypeManager*> managers;

public:
	void AddType(const GTSL::Ranger<char>& name, TypeManager* typeManager);

	[[nodiscard]] TypeManager* GetTypeManager(const Entity& entity) const noexcept
	{
		return managers[entity.GetIndex()];
	}

	auto begin() { return managers.begin(); }
	auto end() { return managers.end(); }
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
