#pragma once

#include "Array.hpp"

template <typename ArrayType>
GS_CLASS SArray : Array<ArrayType>
{
public:
	SArray(int N);
	~SArray();

	void SetElement(unsigned int Index, const ArrayType & Object);
	void SetElement(const ArrayType & Object);
	void RemoveElement(unsigned int Index, bool AdjustStack);
	ArrayType & operator[](unsigned int Index);
	ArrayType & operator=(const ArrayType & Other);
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
void SArray<ArrayType>::SetElement(unsigned int Index, const ArrayType & Object)
{
	(* Arrayptr)[Index] = Object;

	return;
}

template <typename ArrayType>
void SArray<ArrayType>::SetElement(const ArrayType & Object)
{
	(* Arrayptr)[ArrayLength] = Object;

	return;
}

template <typename ArrayType>
void SArray<ArrayType>::RemoveElement(unsigned int Index, bool AdjustStack)
{
	(* Arrayptr)[Index] = ArrayType();

	ArrayLength -= 1;
}

template <typename ArrayType>
ArrayType & SArray<ArrayType>::operator[](unsigned int Index)
{
	return (* Arrayptr)[Index];
}

template <typename ArrayType>
ArrayType & SArray<ArrayType>::operator=(const ArrayType & Other)
{
	return Other;
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