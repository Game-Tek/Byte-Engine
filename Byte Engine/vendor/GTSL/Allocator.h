#pragma once

#include "Core.h"

class Allocator
{
public:
	static void* AlignForward(void* address, const uint8 alignment)
	{
		return reinterpret_cast<void*>((reinterpret_cast<uint8>(address) + static_cast<uint8>(alignment - 1)) & static_cast<uint8>(~(alignment - 1)));
	}

	static inline uint8 alignForwardAdjustment(const void* address, const uint8 alignment)
	{
		const uint8 adjustment = alignment - (reinterpret_cast<uint8>(address) & static_cast<uint8>(alignment - 1));

		if (adjustment == alignment) return 0;

		//already aligned 
		return adjustment;
	}

	static inline uint8 alignForwardAdjustmentWithHeader(const void* address, const uint8 alignment, const uint8 headerSize)
	{
		auto adjustment = alignForwardAdjustment(address, alignment);
		auto neededSpace = headerSize;

		if (adjustment < neededSpace)
		{
			neededSpace -= adjustment;

			//Increase adjustment to fit header 
			adjustment += alignment * (neededSpace / alignment);

			if (neededSpace % alignment > 0) adjustment += alignment;
		}
		
		return adjustment;
	}
};

struct AllocatorReference
{
	/**
	 * \brief Allocates memory.
	 * \param size Number of bytes to allocate.
	 * \param alignment Alignment of the allocation.
	 * \param memory Pointer to a pointer to store the allocation.
	 * \param allocatedSize Pointer to store the allocated size, this will be at least as big as size. This is Done as sometimes allocators allocate more space than what it was asked to, this way the client can take advantage of this and less memory is wasted.
	 */
	void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;
	/**
	 * \brief Deallocates allocated memory.
	 * \param size Size of the allocation being freed, can be the original size asked for, not necessarily the returned allocatedSize when allocating.
	 * \param memory Pointer to the memory block being freed.
	 */
	void Deallocate(uint64 size, void* memory) const;
};