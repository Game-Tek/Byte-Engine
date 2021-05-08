#pragma once

#include "Application.h"

#include <GTSL/Allocator.h>
#include "SystemAllocator.h"

static SystemAllocator system_allocator;

struct SystemAllocatorReference : GTSL::AllocatorReference
{
	void Allocate(const uint64 size, const uint64 alignment, void** data, uint64* allocatedSize) const
	{
		*allocatedSize = size;
		system_allocator.Allocate(size, alignment, data);
	}

	void Deallocate(const uint64 size, const uint64 alignment, void* data) const
	{
		system_allocator.Deallocate(size, alignment, data);
	}

	SystemAllocatorReference()
	{}
};

inline SystemAllocatorReference system_allocator_reference;

extern GTSL::SmartPointer<BE::Application, SystemAllocatorReference> CreateApplication(const SystemAllocatorReference&); //Is defined in another translation unit.

inline bool BasicCompatibilityTest() {
	return sizeof(utf8) == 1 && sizeof(uint8) == 1 && sizeof(uint16) == 2 && sizeof(uint32) == 4 && sizeof(uint64) == 8;
}

int main(int argc, char** argv)
{	
	int exitCode = 0;

	if (!BasicCompatibilityTest()) { exitCode = -1; return exitCode; }
	
	//When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	auto application = CreateApplication(system_allocator_reference);

	application->SetSystemAllocator(&system_allocator);

	if (application->BaseInitialize(argc, argv)) //call BE::Application initialize, which does basic universal startup
	{
		if (application->Initialize()) //call BE::Application virtual initialize which will call the chain of initialize's
		{
			application->PostInitialize();
			//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
			exitCode = application->Run(argc, argv);
		}
	}
	
	application->Shutdown();

	return exitCode; //Return and exit.
}
