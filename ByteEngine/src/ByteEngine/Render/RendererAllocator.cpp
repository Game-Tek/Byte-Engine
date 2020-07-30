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

	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	renderDevice.GetBufferMemoryRequirements(&scratch_buffer, buffer_memory_requirements);

	bufferMemoryType = buffer_memory_requirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);

	scratch_buffer.Destroy(&renderDevice);
}

void ScratchMemoryAllocator::AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data, AllocationId* allocId, const BE::PersistentAllocatorReference& allocatorReference)
{
	uint32 i = 0, id = 0;
	
	for(auto& e : bufferMemoryBlocks)
	{
		if(e.TryAllocate(deviceMemory, size, offset, data, id))
		{
			*reinterpret_cast<uint8*>(allocId) = i;
			*(reinterpret_cast<uint32*>(allocId) + 1) = i;
			return;
		}
		
		++i;
	}

	bufferMemoryBlocks.EmplaceBack();
	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);
	bufferMemoryBlocks.back().AllocateFirstBlock(deviceMemory, size, offset, data, id);

	*reinterpret_cast<uint8*>(allocId) = i;
	*(reinterpret_cast<uint32*>(allocId) + 1) = i;
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
	*offset = freeSpaces[0].Offset;
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
	Buffer scratch_buffer(buffer_create_info);

	Image::CreateInfo create_info;
	create_info.RenderDevice = &renderDevice;
	create_info.Extent = { 1280, 720 };
	create_info.Dimensions = GAL::ImageDimensions::IMAGE_2D;
	create_info.ImageUses = ImageUse::TRANSFER_DESTINATION;
	create_info.InitialLayout = GAL::ImageLayout::UNDEFINED;
	create_info.SourceFormat = (uint32)ImageFormat::RGBA_I8;
	create_info.ImageTiling = (uint32)GAL::VulkanImageTiling::OPTIMAL;
	auto image = Image(create_info);

	RenderDevice::ImageMemoryRequirements image_memory_requirements;
	renderDevice.GetImageMemoryRequirements(&image, image_memory_requirements);

	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	renderDevice.GetBufferMemoryRequirements(&scratch_buffer, buffer_memory_requirements);

	bufferMemoryType = buffer_memory_requirements.MemoryTypes;
	textureMemoryType = image_memory_requirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);
	textureMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), textureMemoryType, allocatorReference);

	scratch_buffer.Destroy(&renderDevice);
	image.Destroy(&renderDevice);
}

void LocalMemoryAllocator::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	for(auto& e : bufferMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
	for(auto& e : textureMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
}

void LocalMemoryAllocator::AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, const uint32 size, uint32* offset, AllocationId* allocId, const BE::PersistentAllocatorReference& allocatorReference)
{
	uint32 i = 0, id;
	
	for(auto& e : bufferMemoryBlocks)
	{
		if(e.TryAllocate(deviceMemory, size, offset))
		{
			*reinterpret_cast<uint8*>(allocId) = i;
			return;
		}
		
		++i;
	}

	bufferMemoryBlocks.EmplaceBack();
	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);
	bufferMemoryBlocks.back().Allocate(deviceMemory, size, offset, id);

	*reinterpret_cast<uint8*>(allocId) = i;
}

