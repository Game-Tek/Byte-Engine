#pragma once

#include "Containers/Id.h"

#include "ResourceData.h"

//class ResourceReference
//{
//	friend class ResourceManager;
//	Id resourceName;
//	Id resourceType;
//
//	ResourceReference(const Id& type, const Id& name) : resourceType(type), resourceName(name)
//	{}
//public:
//	[[nodiscard]] Id GetName() const { return resourceName; }
//};

class ResourceReference
{
	friend class ResourceManager;
	Id resourceName;
	Id resourceType;

	ResourceReference(const Id& type, const Id& name) : resourceType(type), resourceName(name)
	{}
public:
	[[nodiscard]] Id GetName() const { return resourceName; }};
