#pragma once

#include "ByteEngine/Core.h"
#include "ByteEngine/Object.h"

class ComponentCollection : public Object
{
public:
	virtual ~ComponentCollection() = default;

	struct CreateInstanceInfo
	{};
	virtual void CreateInstance(const CreateInstanceInfo& createInstanceInfo) = 0;
	
	struct DestroyInstanceInfo
	{};
	virtual void DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo) = 0;

	struct UpdateInstancesInfo
	{};
	virtual void UpdateInstances(const UpdateInstancesInfo& updateInstancesInfo) = 0;
};
