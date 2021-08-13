#include "RendererAllocator.h"

#include "ByteEngine/Debug/Assert.h"

static constexpr uint8 ALLOC_IS_ISOLATED = 0;
static constexpr uint8 IS_PRE_BLOCK_CONTIGUOUS = 1;
static constexpr uint8 IS_POST_BLOCK_CONTIGUOUS = 2;
static constexpr uint8 IS_PRE_AND_POST_BLOCK_CONTIGUOUS = IS_PRE_BLOCK_CONTIGUOUS | IS_POST_BLOCK_CONTIGUOUS;

void ScratchMemoryAllocator::Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	bufferMemoryBlocks.EmplaceBack(GetPersistentAllocator());
	//textureMemoryBlocks.EmplaceBack();

	GPUBuffer scratchBuffer;
	
	GAL::MemoryRequirements memory_requirements;
	scratchBuffer.GetMemoryRequirements(&renderDevice, 1024,
		GAL::BufferUses::UNIFORM | GAL::BufferUses::TRANSFER_SOURCE | GAL::BufferUses::INDEX | GAL::BufferUses::VERTEX | GAL::BufferUses::ADDRESS | GAL::BufferUses::SHADER_BINDING_TABLE,
		&memory_requirements);

	//bufferMemoryType = memory_requirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, ALLOCATION_SIZE, GAL::MemoryTypes::HOST_VISIBLE | GAL::MemoryTypes::HOST_COHERENT, allocatorReference);

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

		bufferMemoryBlocks.EmplaceBack(GetPersistentAllocator());
		bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), GAL::MemoryTypes::HOST_VISIBLE | GAL::MemoryTypes::HOST_COHERENT, GetPersistentAllocator());
		bufferMemoryBlocks.back().Allocate(deviceMemory, alignedSize, allocation, &renderAllocation->Data);
	}
	else
	{
		deviceMemory->Initialize(&renderDevice, GAL::AllocationFlags::DEVICE_ADDRESS, alignedSize, renderDevice.FindNearestMemoryType(GAL::MemoryTypes::HOST_VISIBLE | GAL::MemoryTypes::HOST_COHERENT));
		
		renderAllocation->Data = deviceMemory->Map(&renderDevice, alignedSize, 0);
	}
	
	allocation.Size = alignedSize;

	//BE_LOG_MESSAGE("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset)
}

void ScratchMemoryAllocator::Free(const RenderDevice& renderDevice,	const BE::PersistentAllocatorReference& allocatorReference)
{
	for (auto& e : bufferMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
}

void MemoryBlock::Initialize(const RenderDevice& renderDevice, uint32 size, GAL::MemoryType memoryType, const BE::PersistentAllocatorReference& allocatorReference)
{	
	deviceMemory.Initialize(&renderDevice, GAL::AllocationFlags::DEVICE_ADDRESS, size, renderDevice.FindNearestMemoryType(memoryType));

	if (static_cast<GAL::MemoryType::value_type>(memoryType & GAL::MemoryTypes::HOST_VISIBLE)) {
		mappedMemory = deviceMemory.Map(&renderDevice, size, 0);
	}
	
	freeSpaces.EmplaceBack(size, 0);
}

void MemoryBlock::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	if (mappedMemory) {
		deviceMemory.Unmap(&renderDevice);
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

		//freeSpaces.Insert(i, Space(size, offset));
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
		//freeSpaces.Insert(i, Space(size, offset));
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
	bufferMemoryBlocks.EmplaceBack(GetPersistentAllocator());
	textureMemoryBlocks.EmplaceBack(GetPersistentAllocator());
	
	Texture dummyTexture;

	GAL::MemoryRequirements imageMemoryRequirements;
	dummyTexture.GetMemoryRequirements(&renderDevice, &imageMemoryRequirements, GAL::TextureUses::TRANSFER_DESTINATION, GAL::FORMATS::RGBA_I8,
		{ 1280, 720, 1 }, GAL::Tiling::OPTIMAL, 1);

	GPUBuffer dummyBuffer;
	
	GAL::MemoryRequirements bufferMemoryRequirements;
	dummyBuffer.GetMemoryRequirements(&renderDevice, 1024,
		GAL::BufferUses::UNIFORM | GAL::BufferUses::TRANSFER_DESTINATION | GAL::BufferUses::INDEX | GAL::BufferUses::VERTEX | GAL::BufferUses::ADDRESS | GAL::BufferUses::SHADER_BINDING_TABLE | GAL::BufferUses::ACCELERATION_STRUCTURE | GAL::BufferUses::BUILD_INPUT_READ,
		&bufferMemoryRequirements);

	//bufferMemoryType = bufferMemoryRequirements.MemoryTypes;
	//textureMemoryType = imageMemoryRequirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, ALLOCATION_SIZE, GAL::MemoryTypes::GPU, allocatorReference);
	textureMemoryBlocks.back().Initialize(renderDevice, ALLOCATION_SIZE, GAL::MemoryTypes::GPU, allocatorReference);

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

		bufferMemoryBlocks.EmplaceBack(GetPersistentAllocator());
		bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), GAL::MemoryTypes::GPU, GetPersistentAllocator());
		bufferMemoryBlocks.back().Allocate(deviceMemory, alignedSize, allocation, &dummy);
	}
	else
	{
		deviceMemory->Initialize(&renderDevice, GAL::AllocationFlags::DEVICE_ADDRESS, alignedSize, renderDevice.FindNearestMemoryType(GAL::MemoryTypes::GPU));
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
	
	textureMemoryBlocks.EmplaceBack(GetPersistentAllocator());
	textureMemoryBlocks.back().Initialize(renderDevice, ALLOCATION_SIZE, GAL::MemoryTypes::GPU, GetPersistentAllocator());
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

