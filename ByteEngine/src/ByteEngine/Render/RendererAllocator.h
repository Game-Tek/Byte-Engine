#pragma once
#include <GTSL/DataSizes.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Math.hpp>


#include "RenderTypes.h"
#include "ByteEngine/Core.h"
#include "ByteEngine/Object.h"
#include "ByteEngine/Application/AllocatorReferences.h"

class ScratchMemoryAllocator;

struct Space
{
	Space() = default;
	
	Space(const uint32 size, const uint32 offset) : Size(size), Offset(offset)
	{
	}

	uint32 Size = 0;
	uint32 Offset = 0;
};

struct AllocationInfo : Space
{
	uint32 BlockIndex = 0;
	uint32 BlockInfo = 0;

	AllocationInfo() = default;
};

struct MemoryBlock
{
	MemoryBlock(const BE::PAR& allocator) : freeSpaces(32, allocator) {}
	
	void Initialize(const RenderDevice& renderDevice, uint32 size, GAL::MemoryType memoryType, const BE::PersistentAllocatorReference& allocatorReference);
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

	bool TryAllocate(DeviceMemory* deviceMemory, uint32 size, AllocationInfo& allocationInfo, void** data);
	void Allocate(DeviceMemory* deviceMemory, uint32 size, AllocationInfo& allocationInfo, void** data);
	void Deallocate(uint32 size, uint32 offset, AllocationInfo id);

private:
	DeviceMemory deviceMemory;
	void* mappedMemory = nullptr;

	GTSL::Vector<Space, BE::PersistentAllocatorReference> freeSpaces;
};

class LocalMemoryAllocator : public Object
{
public:
	LocalMemoryAllocator() : Object(u8"LocalMemoryAllocator") {}

	void Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

	void AllocateLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, uint32 size, uint32* offset);
	
	void DeallocateLinearMemory(const RenderDevice& renderDevice, const RenderAllocation renderAllocation)
	{
		if constexpr (!SINGLE_ALLOC)
		{
			auto& allocation = allocations[renderAllocation.AllocationId];
			bufferMemoryBlocks[allocation.BlockIndex].Deallocate(GTSL::Math::RoundUpByPowerOf2(allocation.Size, granularity), allocation.Offset, allocation);
		}
	}

	void AllocateNonLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, uint32 size, uint32* offset);
	void DeallocateNonLinearMemory(const RenderDevice& renderDevice, const RenderAllocation renderAllocation)
	{
		if constexpr (!SINGLE_ALLOC)
		{
			auto& allocation = allocations[renderAllocation.AllocationId];
			textureMemoryBlocks[allocation.BlockIndex].Deallocate(GTSL::Math::RoundUpByPowerOf2(allocation.Size, granularity), allocation.Offset, allocation);
		}
	}

private:
	static constexpr GTSL::Byte ALLOCATION_SIZE{ GTSL::MegaByte(128) };

	static constexpr bool SINGLE_ALLOC = false;
	
	uint32 bufferMemoryType = 0, textureMemoryType = 0;

	
	GTSL::Array<AllocationInfo, 32> allocations;
	
	GTSL::Array<MemoryBlock, 32> bufferMemoryBlocks;
	GTSL::Array<MemoryBlock, 32> textureMemoryBlocks;
	uint32 bufferMemoryAlignment = 0, textureMemoryAlignment = 0;
	GTSL::uint32 granularity;
};

class ScratchMemoryAllocator : public Object
{
public:
	ScratchMemoryAllocator() : Object(u8"ScratchMemoryAllocator") {}

	void Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);
	
	void AllocateLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, uint32 size, uint32* offset);
	void DeallocateLinearMemory(const RenderDevice& renderDevice, const RenderAllocation renderAllocation)
	{
		if constexpr (!SINGLE_ALLOC)
		{
			auto& allocation = allocations[renderAllocation.AllocationId];
			bufferMemoryBlocks[allocation.BlockIndex].Deallocate(GTSL::Math::RoundUpByPowerOf2(allocation.Size, granularity), allocation.Offset, allocation);
		}
	}
	
	void Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference);

private:
	static constexpr GTSL::Byte ALLOCATION_SIZE{ GTSL::MegaByte(128) };

	static constexpr bool SINGLE_ALLOC = false;
	
	uint32 bufferMemoryType = 0;

	uint32 bufferMemoryAlignment = 0;

	GTSL::uint32 granularity;

	GTSL::Array<AllocationInfo, 32> allocations;
	GTSL::Array<MemoryBlock, 32> bufferMemoryBlocks;
};