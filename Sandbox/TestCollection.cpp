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

void TestCollection::DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo)
{
}

void TestCollection::UpdateInstances(const UpdateInstancesInfo& updateInstancesInfo)
{
}
