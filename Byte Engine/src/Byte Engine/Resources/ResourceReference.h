#pragma once

#include <GTSL/Id.h>

#include "ResourceData.h"

//class ResourceReference
//{
//	friend class ResourceManager;
//	GTSL::Id64 resourceName;
//	GTSL::Id64 resourceType;
//
//	ResourceReference(const GTSL::Id64& type, const GTSL::Id64& name) : resourceType(type), resourceName(name)
//	{}
//public:
//	[[nodiscard]] GTSL::Id64 GetName() const { return resourceName; }
//};

class ResourceReference
{
	friend class ResourceManager;
	GTSL::Id64 resourceName;
	GTSL::Id64 resourceType;

	
	ResourceReference(const GTSL::Id64& type, const GTSL::Id64& name) : resourceType(type), resourceName(name)
	{}
public:
	ResourceReference() = default;
	
	[[nodiscard]] GTSL::Id64 GetName() const { return resourceName; }};
