#include "RendererAllocator.h"

#include "ByteEngine/Debug/Assert.h"

static constexpr uint8 ALLOC_IS_ISOLATED = 0;
static constexpr uint8 IS_PRE_BLOCK_CONTIGUOUS = 1;
static constexpr uint8 IS_POST_BLOCK_CONTIGUOUS = 2;
static constexpr uint8 IS_PRE_AND_POST_BLOCK_CONTIGUOUS = IS_PRE_BLOCK_CONTIGUOUS | IS_POST_BLOCK_CONTIGUOUS;

void ScratchMemoryAllocator::Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	bufferMemoryBlocks.EmplaceBack();
	//textureMemoryBlocks.EmplaceBack();

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.Size = 1024;
	buffer_create_info.BufferType = BufferType::UNIFORM | BufferType::TRANSFER_SOURCE | BufferType::INDEX | BufferType::VERTEX;
	Buffer scratch_buffer(buffer_create_info);

	RenderDevice::MemoryRequirements memory_requirements;
	renderDevice.GetBufferMemoryRequirements(&scratch_buffer, memory_requirements);

	bufferMemoryType = memory_requirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);

	bufferMemoryAlignment = memory_requirements.Alignment;
	
	scratch_buffer.Destroy(&renderDevice);
}

void ScratchMemoryAllocator::AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, uint32 size, HostRenderAllocation* renderAllocation, const BE::PersistentAllocatorReference& allocatorReference)
{
	AllocID allocationId;
	
	const auto alignedSize = GTSL::Math::PowerOf2RoundUp(size, bufferMemoryAlignment);
	
	for (auto& e : bufferMemoryBlocks)
	{
		if (e.TryAllocate(deviceMemory, alignedSize, &renderAllocation->Offset, &renderAllocation->Data, allocationId.BlockInfo))
		{
			renderAllocation->Size = alignedSize;
			renderAllocation->AllocationId = allocationId;
			
			return;
		}
	
		++allocationId.Index;
	}
	
	bufferMemoryBlocks.EmplaceBack();
	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);
	bufferMemoryBlocks.back().AllocateFirstBlock(deviceMemory, alignedSize, &renderAllocation->Offset, &renderAllocation->Data, allocationId.BlockInfo);

	//{
	//	DeviceMemory::CreateInfo memory_create_info;
	//	memory_create_info.RenderDevice = &renderDevice;
	//	memory_create_info.Name = "Buffer Shared Memory Block";
	//	memory_create_info.Size = size;
	//	memory_create_info.MemoryType = renderDevice.FindMemoryType(bufferMemoryType, MemoryType::SHARED | MemoryType::COHERENT);
	//	*deviceMemory = DeviceMemory(memory_create_info);
	//
	//	DeviceMemory::MapInfo map_info;
	//	map_info.RenderDevice = &renderDevice;
	//	map_info.Size = memory_create_info.Size;
	//	map_info.Offset = 0;
	//	renderAllocation->Data = deviceMemory->Map(map_info);
	//}

	renderAllocation->Size = alignedSize;
	renderAllocation->AllocationId = allocationId;
}

void ScratchMemoryAllocator::Free(const RenderDevice& renderDevice,	const BE::PersistentAllocatorReference& allocatorReference)
{
	for (auto& e : bufferMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
}

void ScratchMemoryBlock::Initialize(const RenderDevice& renderDevice, const uint32 size, const uint32 memType, const BE::PersistentAllocatorReference& allocatorReference)
{
	freeSpaces.Initialize(16, allocatorReference);
	
	DeviceMemory::CreateInfo memory_create_info;
	memory_create_info.RenderDevice = &renderDevice;
	memory_create_info.Name = "Buffer Shared Memory Block";
	memory_create_info.Size = size;
	memory_create_info.MemoryType = renderDevice.FindMemoryType(memType, MemoryType::SHARED | MemoryType::COHERENT);
	::new(&deviceMemory) DeviceMemory(memory_create_info);

	DeviceMemory::MapInfo map_info;
	map_info.RenderDevice = &renderDevice;
	map_info.Size = memory_create_info.Size;
	map_info.Offset = 0;
	mappedMemory = deviceMemory.Map(map_info);
	
	freeSpaces.EmplaceBack(size, 0);
}

void ScratchMemoryBlock::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	DeviceMemory::UnmapInfo unmap_info;
	unmap_info.RenderDevice = &renderDevice;
	deviceMemory.Unmap(unmap_info);

	deviceMemory.Destroy(&renderDevice);
}

bool ScratchMemoryBlock::TryAllocate(DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data, uint32& id)
{
	uint32 i = 0;
	
	for (auto& e : freeSpaces)
	{
		if (e.Size >= size)
		{
			*data = static_cast<byte*>(mappedMemory) + e.Offset;
			*offset = e.Offset;
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

void ScratchMemoryBlock::AllocateFirstBlock(DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data, uint32& id)
{
	*data = static_cast<byte*>(mappedMemory) + freeSpaces[0].Offset;
	*offset = freeSpaces[0].Offset;
	*deviceMemory = this->deviceMemory;

	freeSpaces[0].Size -= size;
	freeSpaces[0].Offset += size;
}

void ScratchMemoryBlock::Deallocate(const uint32 size, const uint32 offset, uint32 id)
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

		freeSpaces.Insert(i, FreeSpace(size, offset));
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
		freeSpaces.Insert(i, FreeSpace(size, offset));
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



void LocalMemoryBlock::Initialize(const RenderDevice& renderDevice, uint32 size, const uint32 memType, const BE::PersistentAllocatorReference& allocatorReference)
{
	freeSpaces.Initialize(16, allocatorReference);

	DeviceMemory::CreateInfo memory_create_info;
	memory_create_info.RenderDevice = &renderDevice;
	memory_create_info.Name = "GPU Memory Block";
	memory_create_info.Size = size;
	memory_create_info.MemoryType = renderDevice.FindMemoryType(memType, static_cast<uint32>(MemoryType::GPU));
	::new(&deviceMemory) DeviceMemory(memory_create_info);

	freeSpaces.EmplaceBack(size, 0);
}

void LocalMemoryBlock::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	deviceMemory.Destroy(&renderDevice);
}

bool LocalMemoryBlock::TryAllocate(DeviceMemory* deviceMemory, const uint32 size, uint32* offset)
{
	uint32 i = 0;

	for (auto& e : freeSpaces)
	{
		if (e.Size >= size)
		{
			*offset = e.Offset;
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

void LocalMemoryBlock::Allocate(DeviceMemory* deviceMemory, const uint32 size, uint32* offset, uint32& id)
{
	*offset = 0;
	*deviceMemory = this->deviceMemory;

	freeSpaces[0].Size -= size;
	freeSpaces[0].Offset += size;
}

void LocalMemoryBlock::Deallocate(const uint32 size, const uint32 offset, uint32 id)
{
	uint8 info = 0; uint32 i = 0;

	if (freeSpaces[0].Offset > offset)
	{
		if (size + offset == freeSpaces[0].Offset) //is post block contiguous
		{
			freeSpaces[i].Size += size;
			freeSpaces[i].Offset = offset;
			return;
		}

		freeSpaces.Insert(i, FreeSpace(size, offset));
		return;
	}

	++i;

	for (; i < freeSpaces.GetLength(); ++i)
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
		freeSpaces.Insert(i, FreeSpace(size, offset));
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

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.Size = 1024;
	buffer_create_info.BufferType = BufferType::UNIFORM | BufferType::TRANSFER_DESTINATION | BufferType::INDEX | BufferType::VERTEX;
	Buffer dummyBuffer(buffer_create_info);

	Texture::CreateInfo create_info;
	create_info.RenderDevice = &renderDevice;
	create_info.Extent = { 1280, 720, 1 };
	create_info.Dimensions = Dimensions::SQUARE;
	create_info.Uses = TextureUses::TRANSFER_DESTINATION;
	create_info.InitialLayout = TextureLayout::UNDEFINED;
	create_info.Format = TextureFormat::RGBA_I8;
	create_info.Tiling = TextureTiling::OPTIMAL;
	Texture dummyTexture(create_info);

	RenderDevice::MemoryRequirements imageMemoryRequirements;
	renderDevice.GetImageMemoryRequirements(&dummyTexture, imageMemoryRequirements);

	RenderDevice::MemoryRequirements bufferMemoryRequirements;
	renderDevice.GetBufferMemoryRequirements(&dummyBuffer, bufferMemoryRequirements);

	bufferMemoryType = bufferMemoryRequirements.MemoryTypes;
	textureMemoryType = imageMemoryRequirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);
	textureMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), textureMemoryType, allocatorReference);

	dummyBuffer.Destroy(&renderDevice);
	dummyTexture.Destroy(&renderDevice);

	bufferMemoryAlignment = bufferMemoryRequirements.Alignment;
	textureMemoryAlignment = imageMemoryRequirements.Alignment;
}

void LocalMemoryAllocator::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	for(auto& e : bufferMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
	for(auto& e : textureMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
}

void LocalMemoryAllocator::AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, const BE::PersistentAllocatorReference& allocatorReference)
{
	AllocID allocId;

	const auto alignedSize = GTSL::Math::PowerOf2RoundUp(renderAllocation->Size, bufferMemoryAlignment);
	
	for(auto& block : bufferMemoryBlocks)
	{
		//TODO: GET BLOCK INFO
		if(block.TryAllocate(deviceMemory, alignedSize, &renderAllocation->Offset))
		{
			renderAllocation->Size = alignedSize;
			renderAllocation->AllocationId = allocId;
			return;
		}
		
		++allocId.Index;
	}
	
	bufferMemoryBlocks.EmplaceBack();
	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);
	bufferMemoryBlocks.back().Allocate(deviceMemory, alignedSize, &renderAllocation->Offset, allocId.BlockInfo);

	//{
	//	DeviceMemory::CreateInfo memory_create_info;
	//	memory_create_info.RenderDevice = &renderDevice;
	//	memory_create_info.Name = "Buffer GPU Memory Block";
	//	memory_create_info.Size = alignedSize;
	//	memory_create_info.MemoryType = renderDevice.FindMemoryType(bufferMemoryType, MemoryType::GPU);
	//	*deviceMemory = DeviceMemory(memory_create_info);
	//}
	
	renderAllocation->Size = alignedSize;
	renderAllocation->AllocationId = allocId;
}

void LocalMemoryAllocator::AllocateTexture(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, const BE::PersistentAllocatorReference& persistentAllocatorReference)
{
	AllocID allocId;

	const auto alignedSize = GTSL::Math::PowerOf2RoundUp(renderAllocation->Size, textureMemoryAlignment);

	for (auto& block : textureMemoryBlocks)
	{
		//TODO: GET BLOCK INFO
		if (block.TryAllocate(deviceMemory, alignedSize, &renderAllocation->Offset))
		{
			renderAllocation->Size = alignedSize;
			renderAllocation->AllocationId = allocId;
			return;
		}
	
		++allocId.Index;
	}
	
	textureMemoryBlocks.EmplaceBack();
	textureMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), textureMemoryType, persistentAllocatorReference);
	textureMemoryBlocks.back().Allocate(deviceMemory, alignedSize, &renderAllocation->Offset, allocId.BlockInfo);

	//{
	//	DeviceMemory::CreateInfo memory_create_info;
	//	memory_create_info.RenderDevice = &renderDevice;
	//	memory_create_info.Name = "Texture GPU Memory Block";
	//	memory_create_info.Size = alignedSize;
	//	memory_create_info.MemoryType = renderDevice.FindMemoryType(textureMemoryType, MemoryType::GPU);
	//	*deviceMemory = DeviceMemory(memory_create_info);
	//}
	
	renderAllocation->Size = alignedSize;
	renderAllocation->AllocationId = allocId;
}

