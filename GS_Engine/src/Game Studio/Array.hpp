#pragma once

#include "Core.h"

template <typename ArrayType>
GS_CLASS Array
{
public:
	void SetElement(unsigned int Index, const ArrayType & Object);
	void SetElement(const ArrayType & Object);
	void RemoveElement(unsigned int Index, bool AdjustStack);

	ArrayType & operator[](unsigned int Index) { return[Index]; }
	ArrayType & operator=(const ArrayType & Other) { return Other; }

	unsigned short GetArrayLength() const { return ArrayLength; }
	unsigned short GetLastIndex() const { return ArrayLength - 1; }
	ArrayType * GetArrayPointer() const { return Arrayptr; }

protected:
	//Pointer to the array.
	ArrayType * Arrayptr;			

	//Number of elements the array currently holds occupied. NOT BASE 0.
	unsigned short ArrayLength;
	//The size of the array including the unnocuppied elements. NOT BASE 0.
	unsigned short ArrayCapacity;
};