#include "RendererAllocator.h"

#include <GTSL/Math/Math.hpp>

void ScratchMemoryAllocator::Init(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	bufferMemoryBlocks.EmplaceBack();
	//textureMemoryBlocks.EmplaceBack();

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.Size = 1024;
	buffer_create_info.BufferType = (uint32)BufferType::UNIFORM | (uint32)BufferType::TRANSFER_SOURCE | (uint32)BufferType::INDEX | (uint32)BufferType::VERTEX;
	Buffer scratch_buffer(buffer_create_info);

	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	renderDevice.GetBufferMemoryRequirements(&scratch_buffer, buffer_memory_requirements);

	bufferMemoryType = buffer_memory_requirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);

	scratch_buffer.Destroy(&renderDevice);
}

void ScratchMemoryAllocator::AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data, const BE::PersistentAllocatorReference& allocatorReference)
{
	for(auto& e : bufferMemoryBlocks)
	{
		if(e.TryAllocate(deviceMemory, size, offset, data)) { return; }
	}

	bufferMemoryBlocks.EmplaceBack();
	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);
	bufferMemoryBlocks.back().AllocateFirstBlock(deviceMemory, size, offset, data);
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
	memory_create_info.MemoryType = renderDevice.FindMemoryType(memType, static_cast<uint32>(MemoryType::SHARED) | static_cast<uint32>(MemoryType::COHERENT));
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

bool ScratchMemoryBlock::TryAllocate(DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data)
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

void ScratchMemoryBlock::AllocateFirstBlock(DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data)
{
	*data = static_cast<byte*>(mappedMemory) + freeSpaces[0].Offset;
	*offset = freeSpaces[0].Offset;
	*deviceMemory = this->deviceMemory;

	freeSpaces[0].Size -= size;
	freeSpaces[0].Offset += size;
}

void ScratchMemoryBlock::Deallocate(const uint32 size, const uint32 offset)
{
	uint32 free_space_index = 0;
	uint32 lowest_free_space_index = 0;
	for (FreeSpace& free_space : freeSpaces)
	{
		uint32 n = free_space_index;

		if (offset <= free_space.Offset) { lowest_free_space_index = n; }

		if (offset + size == free_space.Offset) //this is free space next to allocation
		{
			free_space.Size += size;
			free_space.Offset = offset;

			if (free_space_index > 0) [[likely]]
			{
				if (freeSpaces[n - 1].Offset + freeSpaces[n - 1].Size == free_space.Offset) //if is contiguous
				{
					FreeSpace prev = freeSpaces[n - 1];
					freeSpaces.Pop(n);
					free_space.Size += prev.Size;
					free_space.Offset = prev.Offset;
				}
			}

			if (free_space_index != freeSpaces.GetLength() - 1) [[likely]]
			{
				if (free_space.Offset + free_space.Size == freeSpaces[n + 1].Offset) //if is contiguous
				{
					FreeSpace next = freeSpaces[n + 1];
					freeSpaces.Pop(n + 1);
					free_space.Size += next.Size;
				}
			}

			return;
		}

		++free_space_index;
	}

	freeSpaces.Insert(lowest_free_space_index, FreeSpace(size, offset));
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

void LocalMemoryBlock::Allocate(DeviceMemory* deviceMemory, const uint32 size, uint32* offset)
{
	*offset = freeSpaces[0].Offset;
	*deviceMemory = this->deviceMemory;

	freeSpaces[0].Size -= size;
	freeSpaces[0].Offset += size;
}

void LocalMemoryBlock::Deallocate(const uint32 size, const uint32 offset)
{
	uint32 free_space_index = 0;
	uint32 lowest_free_space_index = 0;
	for (FreeSpace& free_space : freeSpaces)
	{
		const uint32 n = free_space_index;

		if (offset <= free_space.Offset) { lowest_free_space_index = n; }

		if (offset + size == free_space.Offset) //this is free space next to allocation
		{
			free_space.Size += size;
			free_space.Offset = offset;

			if (free_space_index > 0) [[likely]]
			{
				if (freeSpaces[n - 1].Offset + freeSpaces[n - 1].Size == free_space.Offset) //if is contiguous
				{
					FreeSpace prev = freeSpaces[n - 1];
					freeSpaces.Pop(n);
					free_space.Size += prev.Size;
					free_space.Offset = prev.Offset;
				}
			}

			if (free_space_index != freeSpaces.GetLength() - 1) [[likely]]
			{
				if (free_space.Offset + free_space.Size == freeSpaces[n + 1].Offset) //if is contiguous
				{
					FreeSpace next = freeSpaces[n + 1];
					freeSpaces.Pop(n + 1);
					free_space.Size += next.Size;
				}
			}

			return;
		}

		++free_space_index;
	}

	freeSpaces.Insert(lowest_free_space_index, FreeSpace(size, offset));
}

void LocalMemoryAllocator::Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	bufferMemoryBlocks.EmplaceBack();
	textureMemoryBlocks.EmplaceBack();

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.Size = 1024;
	buffer_create_info.BufferType = (uint32)BufferType::UNIFORM | (uint32)BufferType::TRANSFER_DESTINATION | (uint32)BufferType::INDEX | (uint32)BufferType::VERTEX;
	Buffer scratch_buffer(buffer_create_info);

	Image::CreateInfo create_info;
	create_info.RenderDevice = &renderDevice;
	create_info.Extent = { 1280, 720 };
	create_info.Dimensions = GAL::ImageDimensions::IMAGE_2D;
	create_info.ImageUses = (uint32)ImageUse::TRANSFER_DESTINATION;
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

void LocalMemoryAllocator::AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, const uint32 size, uint32* offset, const BE::PersistentAllocatorReference& allocatorReference)
{
	for(auto& e : bufferMemoryBlocks)
	{
		if(e.TryAllocate(deviceMemory, size, offset)) { return; }
	}

	bufferMemoryBlocks.EmplaceBack();
	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, allocatorReference);
	bufferMemoryBlocks.back().Allocate(deviceMemory, size, offset);
}
