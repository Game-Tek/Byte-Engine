#pragma once

#include "Core.h"

//
// ALLOCATOR
//  |-POOL
//    |-BLOCK
//      |-SLOT
//

class MainAllocator
{
	class Pool
	{
		struct Block
		{
			const size_t blockSize{ 0 };
			
			void* allocation{ nullptr };
			void* end{ nullptr };
			
			
			//If true block is free.
			//Vector<bool> slots;
			//
			//Block(uint64 size, uint64 alignment)
			//{
			//}
			//
			bool IsPointerInBlock(void* p) const { return p > allocation && p < end; }
			char SlotIndexFromPointer(void* p) const
			{
				BE_ASSERT(p > end || p < allocation, "p does not belong to block!");
				return (static_cast<char*>(end) - static_cast<char*>(p)) / blockSize;
			}

			//find first free slot, mark it as allocated
			
			//slots[SlotIndexFromPointer] = true;
		};

		//Vector<Block>
	public:
		//void Deallocate()
	};

	
	
	//Vector<> pools;
public:
	void Allocate()
	{
		//size to power of two
		//power of two to index
		//index pool
		//check if new block has to be created
		//get first compatible block
		//find free slot mark as occupied
	}
	
	void Deallocate()
	{
		//size to power of two
		//power of two to index
		//index pool
		//find block in pool that owns pointer
		//index slot from pointer
		//mark slot as free
	}
};
