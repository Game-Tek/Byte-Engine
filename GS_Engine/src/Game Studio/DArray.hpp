#pragma once

#include "Array.hpp"

template <typename ArrayType>
GS_CLASS DArray : public Array<ArrayType>
{
public:
	DArray(int N);
	~DArray();

	void AddElement(int Index, const ArrayType & Element);
	void RemoveElement(int Index, bool AdjustStack);
private:
	const int DEFAULT_ARRAY_SIZE = 5;

	ArrayType * AllocateNewArray(int N);
	void FillArray(ArrayType ArrayToFill[], ArrayType SourceArray[]);
};

template <typename ArrayType>
DArray<ArrayType>::DArray(int N)
{
	this->Arrayptr = AllocateNewArray((N < this->DEFAULT_ARRAY_SIZE) ? this->DEFAULT_ARRAY_SIZE : N);
}

template <typename ArrayType>
DArray<ArrayType>::~DArray()
{
	delete[] this->Arrayptr;
}

template <typename ArrayType>
void DArray<ArrayType>::AddElement(int Index, const ArrayType & Element)
{
	if (this->NumberOfElements + 1 > this->TotalNumberOfElements)										//We check if adding a new element will exceed the allocated elements.
	{
		ArrayType * NewArray = AllocateNewArray(this->NumberOfElements + 1 + DEFAULT_ARRAY_SIZE);		//We allocate a new array to a temp pointer.

		FillArray(NewArray, Arrayptr);																	//We fill the new array with the contents of the old/current one.

		delete[] this->Arrayptr;																		//We delete the old array which Arrayptr is pointing to.

		this->Arrayptr = NewArray;																		//We set the Array pointer to the recently created and filled array.

		this->TotalNumberOfElements = this->NumberOfElements + 1 + this->DEFAULT_ARRAY_SIZE;			//We update the total number of elements count.
	}

	else
	{
		this->Arrayptr[this->NumberOfElements] = Element;												//We set the last index + 1 as the Element parameter.
	}

	this->NumberOfElements += 1;																		//We update the number of elements count.

	return;
}

template <typename ArrayType>
void DArray<ArrayType>::RemoveElement(int Index, bool AdjustStack)
{
	if (AdjustStack)
	{
		for (unsigned short i = Index; i < GetArrayLength(); i++)
		{

		}
	}

	this->Arrayptr[Index] = ArrayType();

	return;
}

template <typename ArrayType>
ArrayType * DArray<ArrayType>::AllocateNewArray(int N)
{
	return new ArrayType[N];
}

template <typename ArrayType>
void DArray<ArrayType>::FillArray(ArrayType ArrayToFill[], ArrayType SourceArray[])
{
	for (unsigned short i = 0; i < this->NumberOfElements; i++)
	{
		ArrayToFill[i] = SourceArray[i];
	}

	return;
}