#pragma once

#include "ByteEngine/Object.h"

class ComponentCollection : public Object
{
public:
	virtual ~ComponentCollection() = default;

	using ComponentReference = uint32;
	
	struct CreateInstanceInfo
	{};
	virtual ComponentReference CreateInstance(const CreateInstanceInfo& createInstanceInfo) = 0;
	
	struct DestroyInstanceInfo
	{};
	virtual void DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo) = 0;
};
