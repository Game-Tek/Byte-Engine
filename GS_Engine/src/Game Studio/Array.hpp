#pragma once

#include "Core.h"

template <typename ArrayType>
GS_CLASS Array
{
public:
	//Sets the specified element as the object.
	void SetElement(unsigned int Index, const ArrayType & Object);

	//Places the object after the last occupied element.
	virtual void PopBack(const ArrayType & Object) = 0 {};

	//Places the object on the first free element found.
	void PopOnFree(const ArrayType & Object)
	{
		unsigned int FirstIndex = FindFirstFreeSlot();

		this->Arrayptr[FirstIndex];

		if (FirstIndex == this->LastIndex)
		{
			this->LastIndex++;
		}
	}

	//Removes the specified element.
	virtual void RemoveElement(unsigned int Index)
	{
		this->Arrayptr[Index] = ArrayType();

		this->ArrayLength--;

		if (Index == this->LastIndex)
		{
			this->LastIndex--;
		}

		return;
	}

	ArrayType & operator[](unsigned int Index) { return this->Arrayptr[Index]; }
	ArrayType operator[](unsigned int Index) const { return this->Arrayptr[Index]; }
	ArrayType & operator=(const ArrayType & Other) { return Other; }

	unsigned short GetArrayLength() const { return this->ArrayLength; }
	unsigned short GetLastIndex() const { return this->LastIndex; }


protected:
	//Pointer to the array.
	ArrayType * Arrayptr = nullptr;			

	//Number of elements the array currently holds occupied. NOT BASE 0.
	unsigned short ArrayLength = 0;

	//The size of the array including the unnocuppied elements. NOT BASE 0.
	unsigned short ArrayCapacity = 0;

	unsigned int LastIndex = 0;

	unsigned int FindFirstFreeSlot()
	{
		ArrayType DefaultValue = ArrayType();

		for (unsigned int i = 0; i < this->ArrayLength; i++)
		{
			if (this->Arrayptr[i] == DefaultValue)
			{
				return i;
			}
		}
	}
};