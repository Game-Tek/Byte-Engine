#pragma once

#include "Containers/Id.h"

#include "ResourceData.h"

//class ResourceReference
//{
//	friend class ResourceManager;
//	Id64 resourceName;
//	Id64 resourceType;
//
//	ResourceReference(const Id64& type, const Id64& name) : resourceType(type), resourceName(name)
//	{}
//public:
//	[[nodiscard]] Id64 GetName() const { return resourceName; }
//};

class ResourceReference
{
	friend class ResourceManager;
	Id64 resourceName;
	Id64 resourceType;

	ResourceReference(const Id64& type, const Id64& name) : resourceType(type), resourceName(name)
	{}
public:
	[[nodiscard]] Id64 GetName() const { return resourceName; }};
