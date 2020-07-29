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
	void Allocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset);
	void Deallocate(uint32 size, uint32 offset);

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

	void AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, uint32 size, uint32* offset, const BE::PersistentAllocatorReference& allocatorReference);
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

	bool TryAllocate(DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data);
	void AllocateFirstBlock(DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data);
	void Deallocate(uint32 size, uint32 offset);
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
	
	void AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data, const BE::PersistentAllocatorReference& allocatorReference);
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	
private:
	static constexpr GTSL::Byte ALLOCATION_SIZE{ GTSL::MegaByte(128) };

	uint32 bufferMemoryType = 0;
	
	GTSL::Array<ScratchMemoryBlock, 32> bufferMemoryBlocks;
};