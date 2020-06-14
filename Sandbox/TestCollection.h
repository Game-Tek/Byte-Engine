#pragma once

#include <ByteEngine/Game/ComponentCollection.h>
#include <GTSL/Vector.hpp>

class TestCollection : public ComponentCollection
{
public:
	TestCollection();
	
	void CreateInstance(const CreateInstanceInfo& createInstanceInfo) override;
	void CreateInstances(const CreateInstancesInfo& createInstancesInfo) override;
	void DestroyInstances(const DestroyInstanceInfo& destroyInstancesInfo) override;
	void DestroyInstances(const DestroyInstancesInfo& destroyInstanceInfo) override;
	void UpdateInstances(const UpdateInstancesInfo& updateInstancesInfo) override;

	GTSL::Vector<float>& GetNumbers() { return numbers; }
	
private:
	GTSL::Vector<float> numbers;
};

