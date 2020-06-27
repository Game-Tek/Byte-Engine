#pragma once

#include <ByteEngine/Game/ComponentCollection.h>
#include <GTSL/Vector.hpp>

class TestCollection : public ComponentCollection
{
public:
	TestCollection();

	uint32 CreateInstance(const CreateInstanceInfo& createInstanceInfo) override;
	void DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo) override;

	GTSL::Vector<float>& GetNumbers() { return numbers; }

	[[nodiscard]] const char* GetName() const override { return "Test Collection"; }
private:
	GTSL::Vector<float> numbers;
};

