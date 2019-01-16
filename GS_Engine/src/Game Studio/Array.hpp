#pragma once

#include "Core.h"

template <typename ArrayType>
GS_CLASS Array
{
public:
	//Sets the specified element as the object.
	void SetElement(unsigned int Index, const ArrayType & Object);

	//Places the object after the last occupied element.
	virtual void PopBack(const ArrayType & Object) = 0;

	//Places the object on the first free element found.
	void PopOnFree(const ArrayType & Object);

	//Removes the specified element.
	virtual void RemoveElement(unsigned int Index) = 0;

	ArrayType & operator[](unsigned int Index) { return[Index]; }
	ArrayType & operator=(const ArrayType & Other) { return Other; }

	unsigned short GetArrayLength() const { return ArrayLength; }
	unsigned short GetLastIndex() const { return LastIndex; }
	ArrayType * GetArrayPointer() const { return Arrayptr; }

	unsigned int FindFirstFreeSlot() const;

protected:
	//Pointer to the array.
	ArrayType * Arrayptr = nullptr;			

	//Number of elements the array currently holds occupied. NOT BASE 0.
	unsigned short ArrayLength = 0;

	//The size of the array including the unnocuppied elements. NOT BASE 0.
	unsigned short ArrayCapacity = 0;

	unsigned int LastIndex = 0;

	unsigned int FindFirstFreeSlot();
};

template <typename ArrayType>
void Array<ArrayType>::SetElement(unsigned int Index, const ArrayType & Object)
{
	this->Arrayptr[Index] = Object;

	if (Index == LastIndex)
	{
		this->LastIndex = Index + 1;
	}

	return;
}

template <typename ArrayType>
unsigned int Array<ArrayType>::FindFirstFreeSlot()
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

template <typename ArrayType>
void Array<ArrayType>::PopOnFree(const ArrayType & Element)
{
	unsigned int FirstIndex = FindFirstFreeSlot();

	this->Arrayptr[FirstIndex];

	if (FirstIndex == this->LastIndex)
	{
		this->LastIndex++;
	}
}


template <typename ArrayType>
void Array<ArrayType>::RemoveElement(unsigned int Index)
{
	this->Arrayptr[Index] = ArrayType();

	this->ArrayLength--;

	if (Index == this->LastIndex)
	{
		LastIndex--;
	}

	return;
}