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
	void Initialize(const RenderDevice& renderDevice, uint32 size, uint32 memType, const BE::PersistentAllocatorReference& allocatorReference);
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

	bool TryAllocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset);
	void Allocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset, uint32& id);
	void Deallocate(uint32 size, uint32 offset, uint32 id);

private:
	DeviceMemory deviceMemory;

	GTSL::Vector<FreeSpace, BE::PersistentAllocatorReference> freeSpaces;
};

class LocalMemoryAllocator
{
public:
	LocalMemoryAllocator() = default;

	void Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

	void AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, uint32 size, uint32* offset,
	                    AllocationId* allocId, const BE::PersistentAllocatorReference& allocatorReference);
	
	void DeallocateBuffer(const RenderDevice& renderDevice, const uint32 size, const uint32 offset, AllocationId allocId)
	{
		uint8* id = reinterpret_cast<uint8*>(&allocId);
		bufferMemoryBlocks[*id].Deallocate(size, offset, *reinterpret_cast<uint32*>(id + 4));
	}
	
private:
	static constexpr GTSL::Byte ALLOCATION_SIZE{ GTSL::MegaByte(128) };
	
	uint32 bufferMemoryType = 0, textureMemoryType = 0;
	
	GTSL::Array<LocalMemoryBlock, 32> bufferMemoryBlocks;
	GTSL::Array<LocalMemoryBlock, 32> textureMemoryBlocks;
};


struct ScratchMemoryBlock
{
	ScratchMemoryBlock() = default;

	void Initialize(const RenderDevice& renderDevice, uint32 size, uint32 memType, const BE::PersistentAllocatorReference& allocatorReference);
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

	bool TryAllocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data, uint32& id);
	void AllocateFirstBlock(DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data, uint32& id);
	void Deallocate(uint32 size, uint32 offset, uint32 id);
private:
	DeviceMemory deviceMemory;
	void* mappedMemory = nullptr;

	GTSL::Vector<FreeSpace, BE::PersistentAllocatorReference> freeSpaces;
};

class ScratchMemoryAllocator
{
public:
	ScratchMemoryAllocator() = default;

	void Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	
	void AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data, AllocationId* allocId, const BE::PersistentAllocatorReference& allocatorReference);
	void DeallocateBuffer(const RenderDevice& renderDevice, uint32 size, uint32 offset, AllocationId allocId)
	{
		uint8* id = reinterpret_cast<uint8*>(&allocId);
		bufferMemoryBlocks[*id].Deallocate(size, offset, *reinterpret_cast<uint32*>(id + 4));
	}
	
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	
private:
	static constexpr GTSL::Byte ALLOCATION_SIZE{ GTSL::MegaByte(128) };

	uint32 bufferMemoryType = 0;
	
	GTSL::Array<ScratchMemoryBlock, 32> bufferMemoryBlocks;
};