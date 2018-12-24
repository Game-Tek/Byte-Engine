#pragma once

#include "Core.h"

template <typename ArrayType>
GS_CLASS Array
{
public:
	void AddElement(int Index, const ArrayType & Element);
	void RemoveElement(int Index, bool AdjustStack);
	unsigned short GetArrayLength();
	unsigned short GetLastIndex();

private:
	ArrayType * Arrayptr;			//Pointer to the array.

	unsigned short ArrayLength;		//Number of elements the array currently holds occupied. NOT BASE 0.
	unsigned short ArrayCapacity;	//The size of the array including the unnocuppied elements. NOT BASE 0.
};