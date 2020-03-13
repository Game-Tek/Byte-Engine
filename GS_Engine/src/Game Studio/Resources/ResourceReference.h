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
	ResourceData* resourceData = 0;

	ResourceReference(const Id& type, const Id& name, ResourceData* resourceData) : resourceType(type), resourceName(name), resourceData(resourceData)
	{}
public:
	[[nodiscard]] Id GetName() const { return resourceName; }
	[[nodiscard]] ResourceData* GetResourceData() const { return resourceData; }
};
