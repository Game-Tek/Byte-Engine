#include "TestCollection.h"

#include "ByteEngine/Core.h"

TestCollection::TestCollection() : numbers(8, GetPersistentAllocator())
{
}

uint32 TestCollection::CreateInstance(const CreateInstanceInfo& createInstanceInfo)
{
	return 0;
}

void TestCollection::DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo)
{
}