#pragma once

#include "Application.h"

#include <GTSL/Allocator.h>

extern BE::Application* BE::CreateApplication(GTSL::AllocatorReference* allocatorReference); //Is defined in another translation unit.
extern void BE::DestroyApplication(BE::Application* application, GTSL::AllocatorReference* allocatorReference); //Is defined in another translation unit.

static SystemAllocator system_allocator;

struct SystemAllocatorReference : GTSL::AllocatorReference
{
protected:
	
	void alloc(uint64 size, uint64 alignment, void** data, uint64* allocatedSize) const
	{
		*allocatedSize = size;
		system_allocator.Allocate(size, alignment, data);
	}

	void dealloc(uint64 size, uint64 alignement, void* data) const
	{
		system_allocator.Deallocate(size, alignement, data);
	}

public:
	SystemAllocatorReference() : AllocatorReference(reinterpret_cast<decltype(allocate)>(&SystemAllocatorReference::alloc), reinterpret_cast<decltype(deallocate)>(&SystemAllocatorReference::dealloc))
	{}
};

inline SystemAllocatorReference system_allocator_reference;

int main(int argc, char** argv)
{	
	//When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	auto application = BE::CreateApplication(&system_allocator_reference);

	application->SetSystemAllocator(&system_allocator);

	application->Initialize();
	//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
	const auto exit_code = application->Run(argc, argv);
	application->Shutdown();
	
	BE::DestroyApplication(application, &system_allocator_reference);

	return exit_code; //Return and exit.
}
