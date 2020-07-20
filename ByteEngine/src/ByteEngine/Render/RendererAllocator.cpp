#include "RendererAllocator.h"

#include <GTSL/Math/Math.hpp>

ScratchMemoryAllocator::ScratchMemoryAllocator(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	bufferMemoryBlocks.EmplaceBack();
	textureMemoryBlocks.EmplaceBack();

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.Size = 1024;
	buffer_create_info.BufferType = (uint32)BufferType::UNIFORM | (uint32)BufferType::TRANSFER_SOURCE | (uint32)BufferType::INDEX | (uint32)BufferType::VERTEX;
	Buffer scratch_buffer(buffer_create_info);

	Image::CreateInfo create_info;
	create_info.RenderDevice = &renderDevice;
	create_info.Extent = { 1280, 720 };
	create_info.Dimensions = GAL::ImageDimensions::IMAGE_2D;
	create_info.ImageUses = (uint32)ImageUse::TRANSFER_SOURCE;
	create_info.InitialLayout = GAL::ImageLayout::UNDEFINED;
	create_info.SourceFormat = (uint32)ImageFormat::RGBA_I8;
	create_info.ImageTiling = (uint32)GAL::VulkanImageTiling::OPTIMAL;
	auto image = Image(create_info);

	RenderDevice::ImageMemoryRequirements image_memory_requirements;
	renderDevice.GetImageMemoryRequirements(&image, image_memory_requirements);
	
	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	renderDevice.GetBufferMemoryRequirements(&scratch_buffer, buffer_memory_requirements);

	bufferMemoryType = buffer_memory_requirements.MemoryTypes;
	
	bufferMemoryBlocks.back().InitBlock(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType);
	textureMemoryBlocks.back().InitBlock(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), image_memory_requirements.MemoryTypes);
}

void ScratchMemoryAllocator::AllocateBuffer(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, uint32 size, uint32* offset, void** data)
{
	for(auto& e : bufferMemoryBlocks)
	{
		if(e.TryAllocate(deviceMemory, size, offset, data)) { return; }
	}

	bufferMemoryBlocks.EmplaceBack();
	bufferMemoryBlocks.back().InitBlock(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType);
	bufferMemoryBlocks.back().Allocate(deviceMemory, size, offset, data);
}

void ScratchMemoryAllocator::Free(const RenderDevice& renderDevice,	const BE::PersistentAllocatorReference& allocatorReference)
{
	for (auto& e : bufferMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
	for (auto& e : textureMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
}

void ScratchMemoryBlock::InitBlock(const RenderDevice& renderDevice, const uint32 size, const uint32 memType)
{
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
			if (i == freeSpaces.GetLength() - 1)
			{
				e.Size -= size;
				return true;
			}

			freeSpaces.EmplaceBack(FreeSpace(e.Size, 0));
			return true;
		}

		++i;
	}


	return false;
}

void ScratchMemoryBlock::Allocate(DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data)
{
	for (auto& e : freeSpaces)
	{
		if (e.Size >= size)
		{
			e.Size -= size;

			*data = static_cast<byte*>(mappedMemory) + e.Offset;
			*offset = e.Offset;
			*deviceMemory = this->deviceMemory;

			e.Offset += size;

			return;
		}
	}
}

void ScratchMemoryBlock::Deallocate(const uint32 size, const uint32 offset)
{
	uint32 free_space_index = 0;
	uint32 low_offset = 0;
	for (FreeSpace& free_space : freeSpaces)
	{
		uint32 n = free_space_index;

		if (offset <= free_space.Offset) { low_offset = n; }

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

	freeSpaces.Insert(low_offset, FreeSpace{ size, offset });
}



void LocalMemoryBlock::InitBlock(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.Size = 1024;
	buffer_create_info.BufferType = (uint32)BufferType::UNIFORM;
	Buffer scratch_buffer(buffer_create_info);

	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	renderDevice.GetBufferMemoryRequirements(&scratch_buffer, buffer_memory_requirements);

	DeviceMemory::CreateInfo memory_create_info;
	memory_create_info.RenderDevice = &renderDevice;
	memory_create_info.Size = static_cast<uint32>(ALLOCATION_SIZE);
	memory_create_info.MemoryType = renderDevice.FindMemoryType(buffer_memory_requirements.MemoryTypes, static_cast<uint32>(MemoryType::GPU));
	::new(&deviceMemory) DeviceMemory(memory_create_info);

	scratch_buffer.Destroy(&renderDevice);

	freeSpaces.EmplaceBack(static_cast<uint32>(ALLOCATION_SIZE), 0);
}

void LocalMemoryBlock::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	deviceMemory.Destroy(&renderDevice);
}

void LocalMemoryAllocator::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	for(auto& e : memoryBlocks) { e.Free(renderDevice, allocatorReference); }
}
