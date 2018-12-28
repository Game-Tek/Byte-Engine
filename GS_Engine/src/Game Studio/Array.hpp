#pragma once

#include "Core.h"

template <typename ArrayType>
GS_CLASS Array
{
public:
	void SetElement(unsigned int Index, const ArrayType & Object);
	void SetElement(const ArrayType & Object);
	void RemoveElement(unsigned int Index, bool AdjustStack);
	ArrayType & operator[](unsigned int Index);
	ArrayType & operator=(const ArrayType & Other);
	unsigned short GetArrayLength();
	unsigned short GetLastIndex();

protected:
	ArrayType * Arrayptr;			//Pointer to the array.

	unsigned short ArrayLength;		//Number of elements the array currently holds occupied. NOT BASE 0.
	unsigned short ArrayCapacity;	//The size of the array including the unnocuppied elements. NOT BASE 0.
};