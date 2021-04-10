#include "RendererAllocator.h"

#include <GTSL/ShortString.hpp>


#include "ByteEngine/Debug/Assert.h"

static constexpr uint8 ALLOC_IS_ISOLATED = 0;
static constexpr uint8 IS_PRE_BLOCK_CONTIGUOUS = 1;
static constexpr uint8 IS_POST_BLOCK_CONTIGUOUS = 2;
static constexpr uint8 IS_PRE_AND_POST_BLOCK_CONTIGUOUS = IS_PRE_BLOCK_CONTIGUOUS | IS_POST_BLOCK_CONTIGUOUS;

void ScratchMemoryAllocator::Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	bufferMemoryBlocks.EmplaceBack();
	//textureMemoryBlocks.EmplaceBack();

	Buffer scratchBuffer;
	
	GAL::MemoryRequirements memory_requirements;
	scratchBuffer.GetMemoryRequirements(&renderDevice, 1024,
		BufferType::UNIFORM | BufferType::TRANSFER_SOURCE | BufferType::INDEX | BufferType::VERTEX | BufferType::ADDRESS | BufferType::SHADER_BINDING_TABLE,
		&memory_requirements);

	bufferMemoryType = memory_requirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, MemoryType::SHARED | MemoryType::COHERENT, allocatorReference);

	bufferMemoryAlignment = memory_requirements.Alignment;
	
	scratchBuffer.Destroy(&renderDevice);

	granularity = renderDevice.GetLinearNonLinearGranularity();
}

void ScratchMemoryAllocator::AllocateLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, uint32 size, uint32* offset)
{
	BE_ASSERT(size > 0 && size <= ALLOCATION_SIZE, "Invalid size!")
	
	const auto alignedSize = GTSL::Math::RoundUpByPowerOf2(size, granularity);

	renderAllocation->AllocationId = allocations.GetLength();
	auto& allocation = allocations.EmplaceBack();
	
	if constexpr (!SINGLE_ALLOC)
	{
		for (auto& e : bufferMemoryBlocks)
		{
			if (e.TryAllocate(deviceMemory, alignedSize, allocation, &renderAllocation->Data))
			{
				allocation.Size = alignedSize;
				*offset = allocation.Offset;
				//BE_LOG_MESSAGE("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset);
				return;
			}

			++allocation.BlockIndex;
		}

		bufferMemoryBlocks.EmplaceBack();
		bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, MemoryType::SHARED | MemoryType::COHERENT, GetPersistentAllocator());
		bufferMemoryBlocks.back().Allocate(deviceMemory, alignedSize, allocation, &renderAllocation->Data);
	}
	else
	{
		DeviceMemory::CreateInfo memory_create_info;
		memory_create_info.RenderDevice = &renderDevice;
		memory_create_info.Name = GTSL::ShortString<64>("Buffer GPU Memory Block");
		memory_create_info.Size = alignedSize;
		memory_create_info.MemoryType = renderDevice.FindMemoryType(bufferMemoryType, MemoryType::SHARED | MemoryType::COHERENT);
		memory_create_info.Flags = AllocationFlags::DEVICE_ADDRESS;
		deviceMemory->Initialize(memory_create_info);

		DeviceMemory::MapInfo map_info;
		map_info.RenderDevice = &renderDevice;
		map_info.Size = memory_create_info.Size;
		map_info.Offset = 0;
		renderAllocation->Data = deviceMemory->Map(map_info);
	}
	
	allocation.Size = alignedSize;

	//BE_LOG_MESSAGE("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset)
}

void ScratchMemoryAllocator::Free(const RenderDevice& renderDevice,	const BE::PersistentAllocatorReference& allocatorReference)
{
	for (auto& e : bufferMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
}

void MemoryBlock::Initialize(const RenderDevice& renderDevice, uint32 size, uint32 memType, MemoryType::value_type memoryType, const BE::PersistentAllocatorReference& allocatorReference)
{
	freeSpaces.Initialize(16, allocatorReference);
	
	DeviceMemory::CreateInfo memory_create_info;
	memory_create_info.RenderDevice = &renderDevice;
	memory_create_info.Name = GTSL::StaticString<32>("Memory Block");
	memory_create_info.Size = size;
	memory_create_info.MemoryType = renderDevice.FindMemoryType(memType, memoryType);
	memory_create_info.Flags = AllocationFlags::DEVICE_ADDRESS;
	deviceMemory.Initialize(memory_create_info);

	if (memoryType & MemoryType::SHARED)
	{
		DeviceMemory::MapInfo map_info;
		map_info.RenderDevice = &renderDevice;
		map_info.Size = memory_create_info.Size;
		map_info.Offset = 0;
		mappedMemory = deviceMemory.Map(map_info);
	}
	
	freeSpaces.EmplaceBack(size, 0);
}

void MemoryBlock::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	if (mappedMemory) {
		DeviceMemory::UnmapInfo unmap_info;
		unmap_info.RenderDevice = &renderDevice;
		deviceMemory.Unmap(unmap_info);
	}

	deviceMemory.Destroy(&renderDevice);
}

bool MemoryBlock::TryAllocate(DeviceMemory* deviceMemory, const uint32 size, AllocationInfo& allocationInfo, void** data)
{
	uint32 i = 0;
	
	for (auto& e : freeSpaces)
	{
		if (e.Size >= size)
		{
			*data = static_cast<byte*>(mappedMemory) + e.Offset;
			allocationInfo.Offset = e.Offset;
			*deviceMemory = this->deviceMemory;
			
			if (e.Size == size)
			{
				freeSpaces.Pop(i);
				return true;
			}
			
			e.Size -= size;
			e.Offset += size;

			return true;
		}

		++i;
	}

	return false;
}

void MemoryBlock::Allocate(DeviceMemory* deviceMemory, const uint32 size, AllocationInfo& allocationInfo, void** data)
{
	*data = static_cast<byte*>(mappedMemory) + freeSpaces[0].Offset;
	allocationInfo.Offset = freeSpaces[0].Offset;
	*deviceMemory = this->deviceMemory;

	freeSpaces[0].Size -= size;
	freeSpaces[0].Offset += size;
}

void MemoryBlock::Deallocate(const uint32 size, const uint32 offset, AllocationInfo id)
{
	uint8 info = 0; uint32 i = 0;

	if (freeSpaces[0].Offset > offset)
	{
		if(size + offset == freeSpaces[0].Offset) //is post block contiguous
		{
			freeSpaces[i].Size += size;
			freeSpaces[i].Offset = offset;
			return;
		}

		freeSpaces.Insert(i, Space(size, offset));
		return;
	}

	++i;

	for(; i < freeSpaces.GetLength(); ++i)
	{
		if (freeSpaces[i].Offset > offset)
		{
			size + offset == freeSpaces[i].Offset ? info |= IS_POST_BLOCK_CONTIGUOUS : 0;
			break;
		}
	}

	freeSpaces[i - 1].Offset + freeSpaces[i - 1].Size == offset ? info |= IS_PRE_BLOCK_CONTIGUOUS : 0;
	
	switch (info)
	{
	case ALLOC_IS_ISOLATED:
		freeSpaces.Insert(i, Space(size, offset));
		return;
		
	case IS_PRE_BLOCK_CONTIGUOUS:
		freeSpaces[i - 1].Size += size;
		return;
		
	case IS_POST_BLOCK_CONTIGUOUS:
		freeSpaces[i].Size += size;
		freeSpaces[i].Offset = offset;
		return;
		
	case IS_PRE_AND_POST_BLOCK_CONTIGUOUS:
		freeSpaces[i - 1].Size += freeSpaces[i].Size + size;
		freeSpaces.Pop(i);
		return;
		
	default: BE_ASSERT(false, "Wa happened?")
	}
}


void LocalMemoryAllocator::Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	bufferMemoryBlocks.EmplaceBack();
	textureMemoryBlocks.EmplaceBack();
	
	Texture dummyTexture;

	GAL::MemoryRequirements imageMemoryRequirements;
	dummyTexture.GetMemoryRequirements(&renderDevice, &imageMemoryRequirements, TextureLayout::UNDEFINED, TextureUse::TRANSFER_DESTINATION, TextureFormat::RGBA_I8,
		{ 1280, 720, 1 }, TextureTiling::OPTIMAL, 1);

	Buffer dummyBuffer;
	
	GAL::MemoryRequirements bufferMemoryRequirements;
	dummyBuffer.GetMemoryRequirements(&renderDevice, 1024,
		BufferType::UNIFORM | BufferType::TRANSFER_DESTINATION | BufferType::INDEX | BufferType::VERTEX | BufferType::ADDRESS | BufferType::SHADER_BINDING_TABLE | BufferType::ACCELERATION_STRUCTURE | BufferType::BUILD_INPUT_READ_ONLY,
		&bufferMemoryRequirements);

	bufferMemoryType = bufferMemoryRequirements.MemoryTypes;
	textureMemoryType = imageMemoryRequirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, MemoryType::GPU, allocatorReference);
	textureMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), textureMemoryType, MemoryType::GPU, allocatorReference);

	dummyBuffer.Destroy(&renderDevice);
	dummyTexture.Destroy(&renderDevice);

	bufferMemoryAlignment = bufferMemoryRequirements.Alignment;
	textureMemoryAlignment = imageMemoryRequirements.Alignment;

	granularity = renderDevice.GetLinearNonLinearGranularity();
}

void LocalMemoryAllocator::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	for(auto& e : bufferMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
	for(auto& e : textureMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
}

void LocalMemoryAllocator::AllocateLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, uint32 size, uint32* offset)
{
	BE_ASSERT(size > 0 && size <= ALLOCATION_SIZE, "Invalid size!")

	const auto alignedSize = GTSL::Math::RoundUpByPowerOf2(size, granularity);

	renderAllocation->AllocationId = allocations.GetLength();
	auto& allocation = allocations.EmplaceBack();
	
	void* dummy;
	
	if constexpr (!SINGLE_ALLOC)
	{
		for (auto& block : bufferMemoryBlocks)
		{
			//TODO: GET BLOCK INFO
			if (block.TryAllocate(deviceMemory, alignedSize, allocation, &dummy))
			{
				allocation.Size = alignedSize;
				*offset = allocation.Offset;
				
				//BE_LOG_MESSAGE("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset);

				return;
			}

			++allocation.BlockIndex;
		}

		bufferMemoryBlocks.EmplaceBack();
		bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, MemoryType::GPU, GetPersistentAllocator());
		bufferMemoryBlocks.back().Allocate(deviceMemory, alignedSize, allocation, &dummy);
	}
	else
	{
		DeviceMemory::CreateInfo memory_create_info;
		memory_create_info.RenderDevice = &renderDevice;
		memory_create_info.Name = GTSL::ShortString<64>("Buffer GPU Memory Block");
		memory_create_info.Size = alignedSize;
		memory_create_info.MemoryType = renderDevice.FindMemoryType(bufferMemoryType, MemoryType::GPU);
		memory_create_info.Flags = AllocationFlags::DEVICE_ADDRESS;
		deviceMemory->Initialize(memory_create_info);
	}
	
	allocation.Size = alignedSize;
	*offset = allocation.Offset;

	//BE_LOG_MESSAGE("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset);
}

void LocalMemoryAllocator::AllocateNonLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, uint32 size, uint32* offset)
{
	const auto alignedSize = GTSL::Math::RoundUpByPowerOf2(size, granularity);

	renderAllocation->AllocationId = allocations.GetLength();
	auto& allocation = allocations.EmplaceBack();
	
	void* dummy;
	
	for (auto& block : textureMemoryBlocks)
	{
		//TODO: GET BLOCK INFO
		if (block.TryAllocate(deviceMemory, alignedSize, allocation, &dummy))
		{
			allocation.Size = alignedSize;
			*offset = allocation.Offset;
			return;
		}
	
		++allocation.BlockIndex;
	}
	
	textureMemoryBlocks.EmplaceBack();
	textureMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), textureMemoryType, MemoryType::GPU, GetPersistentAllocator());
	textureMemoryBlocks.back().Allocate(deviceMemory, alignedSize, allocation, &dummy);

	//{
	//	DeviceMemory::CreateInfo memory_create_info;
	//	memory_create_info.RenderDevice = &renderDevice;
	//	memory_create_info.Name = "Texture GPU Memory Block";
	//	memory_create_info.Size = alignedSize;
	//	memory_create_info.MemoryType = renderDevice.FindMemoryType(textureMemoryType, MemoryType::GPU);
	//	*deviceMemory = DeviceMemory(memory_create_info);
	//}
	
	allocation.Size = alignedSize;
	*offset = allocation.Offset;
}

