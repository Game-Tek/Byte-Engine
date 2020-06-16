#pragma once

#include <ByteEngine/Game/ComponentCollection.h>
#include <GTSL/Vector.hpp>

class TestCollection : public ComponentCollection
{
public:
	TestCollection();
	
	void CreateInstance(const CreateInstanceInfo& createInstanceInfo) override;
	void DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo) override;
	void UpdateInstances(const UpdateInstancesInfo& updateInstancesInfo) override;

	GTSL::Vector<float>& GetNumbers() { return numbers; }

	[[nodiscard]] const char* GetName() const override { return "Test Collection"; }
private:
	GTSL::Vector<float> numbers;
};

