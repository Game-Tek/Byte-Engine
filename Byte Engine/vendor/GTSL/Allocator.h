#pragma once

#include "Core.h"

class Allocator
{
	class Block
	{
		size_t size = 0;
		void* data = nullptr;
		size_t marker = 0;

	public:
		Block(const size_t size, const void* data);
	};

public:
	static void* AlignForward(void* address, const uint8 alignment)
	{
		return reinterpret_cast<void*>((reinterpret_cast<uint8>(address) + static_cast<uint8>(alignment - 1)) &
			static_cast<uint8>(~(alignment - 1)));
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
