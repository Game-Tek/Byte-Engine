#pragma once
#include <GTSL/DataSizes.h>
#include <GTSL/Vector.hpp>


#include "RenderTypes.h"
#include "ByteEngine/Core.h"
#include "ByteEngine/Application/AllocatorReferences.h"

struct FreeSpace
{
	FreeSpace(const uint32 size, const uint32 offset) : Size(size), Offset(offset)
	{
	}

	uint32 Size = 0;
	uint32 Offset = 0;
};

struct LocalMemoryBlock
{
	static constexpr GTSL::Byte ALLOCATION_SIZE{ GTSL::MegaByte(128) };

	void InitBlock(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

	bool TryAllocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset);
	void Allocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset);
	void Deallocate(uint32 size, uint32 offset);

private:
	DeviceMemory deviceMemory;

	GTSL::Vector<FreeSpace, BE::PersistentAllocatorReference> freeSpaces;
};

class LocalMemoryAllocator
{
public:
	LocalMemoryAllocator(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

private:
	GTSL::Array<LocalMemoryBlock, 32> memoryBlocks;
};

struct ScratchMemoryBlock
{
	ScratchMemoryBlock() = default;

	void InitBlock(const RenderDevice& renderDevice, uint32 size, uint32 memType);
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

	bool TryAllocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data);
	void Allocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data);
	void Deallocate(uint32 size, uint32 offset);
private:
	DeviceMemory deviceMemory;
	void* mappedMemory = nullptr;

	GTSL::Vector<FreeSpace, BE::PersistentAllocatorReference> freeSpaces;
};

class ScratchMemoryAllocator
{
public:
	ScratchMemoryAllocator(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

	void AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data);
	
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	
private:
	static constexpr GTSL::Byte ALLOCATION_SIZE{ GTSL::MegaByte(128) };

	uint32 bufferMemoryType = 0;
	
	GTSL::Array<ScratchMemoryBlock, 32> bufferMemoryBlocks;
	GTSL::Array<ScratchMemoryBlock, 32> textureMemoryBlocks;
};