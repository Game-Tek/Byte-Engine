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

int main(int argc, char** argv)
{	
	//When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	auto application = CreateApplication(system_allocator_reference);

	application->SetSystemAllocator(&system_allocator);

	application->Initialize();
	application->PostInitialize();
	//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
	const auto exit_code = application->Run(argc, argv);
	application->Shutdown();

	return exit_code; //Return and exit.
}
