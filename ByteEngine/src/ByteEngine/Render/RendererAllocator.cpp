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

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.Size = 1024;
	buffer_create_info.BufferType = BufferType::UNIFORM | BufferType::TRANSFER_SOURCE | BufferType::INDEX | BufferType::VERTEX | BufferType::ADDRESS | BufferType::SHADER_BINDING_TABLE;
	Buffer scratch_buffer;

	Buffer::GetMemoryRequirementsInfo memory_requirements;
	memory_requirements.CreateInfo = &buffer_create_info;
	memory_requirements.RenderDevice = &renderDevice;
	scratch_buffer.GetMemoryRequirements(&memory_requirements);

	bufferMemoryType = memory_requirements.MemoryRequirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, MemoryType::SHARED | MemoryType::COHERENT, allocatorReference);

	bufferMemoryAlignment = memory_requirements.MemoryRequirements.Alignment;
	
	scratch_buffer.Destroy(&renderDevice);

	granularity = renderDevice.GetLinearNonLinearGranularity();
}

void ScratchMemoryAllocator::AllocateLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, const BE::PersistentAllocatorReference& allocatorReference)
{
	BE_ASSERT(renderAllocation->Size > 0 && renderAllocation->Size <= ALLOCATION_SIZE, "Invalid size!")
	
	AllocID allocationId;
	
	const auto alignedSize = GTSL::Math::RoundUpByPowerOf2(renderAllocation->Size/* + ((!SINGLE_ALLOC) * 1000000)*/, granularity);

	if constexpr (!SINGLE_ALLOC)
	{
		for (auto& e : bufferMemoryBlocks)
		{
			if (e.TryAllocate(deviceMemory, alignedSize, &renderAllocation->Offset, &renderAllocation->Data, allocationId.BlockInfo))
			{
				renderAllocation->Size = alignedSize;
				renderAllocation->AllocationId = allocationId;

				BE_LOG_WARNING("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset)

					return;
			}

			++allocationId.Index;
		}

		bufferMemoryBlocks.EmplaceBack();
		bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, MemoryType::SHARED | MemoryType::COHERENT, allocatorReference);
		bufferMemoryBlocks.back().Allocate(deviceMemory, alignedSize, &renderAllocation->Offset, &renderAllocation->Data, allocationId.BlockInfo);
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
	
	renderAllocation->Size = alignedSize;
	renderAllocation->AllocationId = allocationId;

	BE_LOG_WARNING("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset)
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

bool MemoryBlock::TryAllocate(DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data, uint32& id)
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

void MemoryBlock::Allocate(DeviceMemory* deviceMemory, const uint32 size, uint32* offset, void** data, uint32& id)
{
	*data = static_cast<byte*>(mappedMemory) + freeSpaces[0].Offset;
	*offset = freeSpaces[0].Offset;
	*deviceMemory = this->deviceMemory;

	freeSpaces[0].Size -= size;
	freeSpaces[0].Offset += size;
}

void MemoryBlock::Deallocate(const uint32 size, const uint32 offset, uint32 id)
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


void LocalMemoryAllocator::Initialize(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	bufferMemoryBlocks.EmplaceBack();
	textureMemoryBlocks.EmplaceBack();

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.Size = 1024;
	buffer_create_info.BufferType = BufferType::UNIFORM | BufferType::TRANSFER_DESTINATION | BufferType::INDEX | BufferType::VERTEX | BufferType::ADDRESS | BufferType::SHADER_BINDING_TABLE | BufferType::ACCELERATION_STRUCTURE | BufferType::BUILD_INPUT_READ_ONLY;
	Buffer dummyBuffer;

	Texture::CreateInfo create_info;
	create_info.RenderDevice = &renderDevice;
	create_info.Extent = { 1280, 720, 1 };
	create_info.Dimensions = Dimensions::SQUARE;
	create_info.Uses = TextureUses::TRANSFER_DESTINATION;
	create_info.InitialLayout = TextureLayout::UNDEFINED;
	create_info.Format = TextureFormat::RGBA_I8;
	create_info.Tiling = TextureTiling::OPTIMAL;
	Texture dummyTexture;

	Texture::GetMemoryRequirementsInfo imageMemoryRequirements;
	imageMemoryRequirements.RenderDevice = &renderDevice;
	imageMemoryRequirements.CreateInfo = &create_info;
	dummyTexture.GetMemoryRequirements(&imageMemoryRequirements);

	Buffer::GetMemoryRequirementsInfo bufferMemoryRequirements;
	bufferMemoryRequirements.CreateInfo = &buffer_create_info;
	bufferMemoryRequirements.RenderDevice = &renderDevice;
	dummyBuffer.GetMemoryRequirements(&bufferMemoryRequirements);

	bufferMemoryType = bufferMemoryRequirements.MemoryRequirements.MemoryTypes;
	textureMemoryType = imageMemoryRequirements.MemoryRequirements.MemoryTypes;

	bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, MemoryType::GPU, allocatorReference);
	textureMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), textureMemoryType, MemoryType::GPU, allocatorReference);

	dummyBuffer.Destroy(&renderDevice);
	dummyTexture.Destroy(&renderDevice);

	bufferMemoryAlignment = bufferMemoryRequirements.MemoryRequirements.Alignment;
	textureMemoryAlignment = imageMemoryRequirements.MemoryRequirements.Alignment;

	granularity = renderDevice.GetLinearNonLinearGranularity();
}

void LocalMemoryAllocator::Free(const RenderDevice& renderDevice, const BE::PersistentAllocatorReference& allocatorReference)
{
	for(auto& e : bufferMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
	for(auto& e : textureMemoryBlocks) { e.Free(renderDevice, allocatorReference); }
}

void LocalMemoryAllocator::AllocateLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, const BE::PersistentAllocatorReference& allocatorReference)
{
	BE_ASSERT(renderAllocation->Size > 0 && renderAllocation->Size <= ALLOCATION_SIZE, "Invalid size!")
	
	AllocID allocId;

	const auto alignedSize = GTSL::Math::RoundUpByPowerOf2(renderAllocation->Size, granularity);

	void* dummy;
	
	if constexpr (!SINGLE_ALLOC)
	{
		for (auto& block : bufferMemoryBlocks)
		{
			//TODO: GET BLOCK INFO
			if (block.TryAllocate(deviceMemory, alignedSize, &renderAllocation->Offset, &dummy, allocId.BlockInfo))
			{
				renderAllocation->Size = alignedSize;
				renderAllocation->AllocationId = allocId;

				BE_LOG_WARNING("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset)

					return;
			}

			++allocId.Index;
		}

		bufferMemoryBlocks.EmplaceBack();
		bufferMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), bufferMemoryType, MemoryType::GPU, allocatorReference);
		bufferMemoryBlocks.back().Allocate(deviceMemory, alignedSize, &renderAllocation->Offset, &dummy, allocId.BlockInfo);
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
	
	renderAllocation->Size = alignedSize;
	renderAllocation->AllocationId = allocId;

	BE_LOG_WARNING("Allocation. Size: ", renderAllocation->Size, " Offset: ", renderAllocation->Offset)
}

void LocalMemoryAllocator::AllocateNonLinearMemory(const RenderDevice& renderDevice, DeviceMemory* deviceMemory, RenderAllocation* renderAllocation, const BE::PersistentAllocatorReference& persistentAllocatorReference)
{
	AllocID allocId;

	const auto alignedSize = GTSL::Math::RoundUpByPowerOf2(renderAllocation->Size, granularity);

	void* dummy;
	
	for (auto& block : textureMemoryBlocks)
	{
		//TODO: GET BLOCK INFO
		if (block.TryAllocate(deviceMemory, alignedSize, &renderAllocation->Offset, &dummy, allocId.BlockInfo))
		{
			renderAllocation->Size = alignedSize;
			renderAllocation->AllocationId = allocId;
			return;
		}
	
		++allocId.Index;
	}
	
	textureMemoryBlocks.EmplaceBack();
	textureMemoryBlocks.back().Initialize(renderDevice, static_cast<uint32>(ALLOCATION_SIZE), textureMemoryType, MemoryType::GPU, persistentAllocatorReference);
	textureMemoryBlocks.back().Allocate(deviceMemory, alignedSize, &renderAllocation->Offset, &dummy, allocId.BlockInfo);

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

