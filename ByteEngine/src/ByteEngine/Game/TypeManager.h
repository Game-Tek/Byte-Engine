#pragma once

class TypeManager
{
public:
	virtual ~TypeManager() = default;

	struct CreateInstanceInfo
	{};
	virtual void CreateInstance(const CreateInstanceInfo& createInstanceInfo) = 0;

	struct CreateInstancesInfo
	{};
	virtual void CreateInstances(const CreateInstancesInfo& createInstancesInfo) = 0;
	
	struct DestroyInstanceInfo
	{};
	virtual void DestroyInstances(const DestroyInstanceInfo& destroyInstancesInfo) = 0;

	struct DestroyInstancesInfo
	{};
	virtual void DestroyInstances(const DestroyInstancesInfo& destroyInstanceInfo) = 0;

	struct UpdateInstancesInfo
	{};
	virtual void UpdateInstances(const UpdateInstancesInfo& updateInstancesInfo) = 0;
};