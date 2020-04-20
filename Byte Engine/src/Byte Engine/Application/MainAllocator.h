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
