#pragma once

#include "Array.hpp"

template <typename ArrayType>
GS_CLASS SArray : Array<ArrayType>
{
public:
	SArray(int N);
	~SArray();

	void AddElement(int Index, const ArrayType & Element);
	void RemoveElement(int Index, bool AdjustStack);
	unsigned short GetArrayLength();
	unsigned short GetLastIndex();
};

template <typename ArrayType>
SArray<ArrayType>::SArray(int N)
{
	Arrayptr = new ArrayType[N];
}

template <typename ArrayType>
SArray<ArrayType>::~SArray()
{
	delete[] Arrayptr;
}

template <typename ArrayType>
void SArray<ArrayType>::AddElement(int Index, const ArrayType & Element)
{
	Arrayptr[ArrayLength] = Element;

	return;
}

template <typename ArrayType>
void SArray<ArrayType>::RemoveElement(int Index, bool AdjustStack)
{
	Arrayptr[Index] = ArrayType();

	ArrayLength -= 1;
}

template <typename ArrayType>
unsigned short SArray<ArrayType>::GetArrayLength()
{
	return ArrayLength;
}

template <typename ArrayType>
unsigned short SArray<ArrayType>::GetLastIndex()
{
	return ArrayLength - 1;
}