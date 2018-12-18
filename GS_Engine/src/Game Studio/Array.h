#pragma once

#include "Core.h"

const int DEFAULT_ARRAY_SIZE = 5;

template <typename ArrayType>
GS_CLASS Array
{
public:
	Array(int N);
	~Array();
	void AddElement(int Index, ArrayType Element);
	void RemoveElement(int Index, bool AdjustStack);
	int GetLastIndex();
	int GetArrayLength();

private:
	ArrayType * Arrayptr;																			//Pointer to the array.
	int NumberOfElements;																			//Number of elements the array currently holds occupied. NOT BASE 0.
	int TotalNumberOfElements;																		//The size of the array including the unnocuppied elements. NOT BASE 0.

	ArrayType * AllocateNewArray(int N);
	void FillArray(ArrayType * ArrayToFill, ArrayType * SourceArray);
};

template <typename ArrayType>
Array<ArrayType>::Array(int N)
{
	Arrayptr = AllocateNewArray((N < DEFAULT_ARRAY_SIZE) ? DEFAULT_ARRAY_SIZE : N);
}

template <typename ArrayType>
Array<ArrayType>::~Array()
{
	delete[] Arrayptr;
}


template<typename ArrayType>
inline void Array<ArrayType>::AddElement(int Index, ArrayType Element)
{
	if (NumberOfElements + 1 > TotalNumberOfElements)													//We check if adding a new element will exceed the allocated elements.
	{
		ArrayType * NewArray = AllocateNewArray(NumberOfElements + 1 + DEFAULT_ARRAY_SIZE);				//We allocate a new array to a temp pointer.

		FillArray(NewArray, Array);																		//We fill the new array with the contents of the old/current one.

		delete[] Arrayptr;																				//We delete the old array which Array is pointing to.

		Arrayptr = NewArray;																			//We set the Array pointer to the recently created and filled array.

		TotalNumberOfElements = NumberOfElements + 1 + DEFAULT_ARRAY_SIZE;								//We update the total number of elements count.
	}

	else
	{
		Array[NumberOfElements] = Element;																//We set the last index + 1 as the Element parameter.
	}

	NumberOfElements += 1;																				//We update the number of elements count.

	return;
}

template<typename ArrayType>
inline void Array<ArrayType>::RemoveElement(int Index, bool AdjustStack)
{
	if (AdjustStack)
	{
		for (i = Index; i < GetArrayLength(); i++)
		{

		}
	}
}

template<typename ArrayType>
inline int Array<ArrayType>::GetLastIndex()
{
	return NumberOfElements - 1;
}

template<typename ArrayType>
inline int Array<ArrayType>::GetArrayLength()
{
	return NumberOfElements;
}

template<typename ArrayType>
inline ArrayType * Array<ArrayType>::AllocateNewArray(int N)
{
	return new New[N];
}

template<typename ArrayType>
inline void Array<ArrayType>::FillArray(ArrayType * ArrayToFill, ArrayType * SourceArray)
{
	for (i = 0; i < NumberOfElements, i++)
	{
		ArrayToFill[i] = SourceArray[i];
	}
}