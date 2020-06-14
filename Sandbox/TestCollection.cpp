#include "TestCollection.h"

#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Core.h"
#include "ByteEngine/Application/AllocatorReferences.h"

static BE::PersistentAllocatorReference persistent_allocator("Test Collection");

TestCollection::TestCollection() : numbers(8, &persistent_allocator)
{
}

void TestCollection::CreateInstance(const CreateInstanceInfo& createInstanceInfo)
{
}

void TestCollection::CreateInstances(const CreateInstancesInfo& createInstancesInfo)
{
	for(uint32 i = 0; i < createInstancesInfo.Count; ++i)
	{
		numbers.EmplaceBack(GTSL::Math::fRandom());
	}
}

void TestCollection::DestroyInstances(const DestroyInstanceInfo& destroyInstancesInfo)
{
}

void TestCollection::DestroyInstances(const DestroyInstancesInfo& destroyInstanceInfo)
{
}

void TestCollection::UpdateInstances(const UpdateInstancesInfo& updateInstancesInfo)
{
}
