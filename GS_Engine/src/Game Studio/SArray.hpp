#pragma once

#include "Array.hpp"

template <typename ArrayType>
GS_CLASS SArray : public Array<ArrayType>
{
public:
	SArray(int N);
	~SArray();

	void SetElement(unsigned int Index, const ArrayType & Object);
	void SetElement(const ArrayType & Object);
	void RemoveElement(unsigned int Index, bool AdjustStack);
};

template <typename ArrayType>
SArray<ArrayType>::SArray(int N)
{
	this->Arrayptr = new ArrayType[N];
}

template <typename ArrayType>
SArray<ArrayType>::~SArray()
{
	delete[] this->Arrayptr;
}

template <typename ArrayType>
void SArray<ArrayType>::SetElement(unsigned int Index, const ArrayType & Object)
{
	this->Arrayptr[Index] = Object;

	return;
}

template <typename ArrayType>
void SArray<ArrayType>::SetElement(const ArrayType & Object)
{
	this->Arrayptr[this->ArrayLength] = Object;

	return;
}

template <typename ArrayType>
void SArray<ArrayType>::RemoveElement(unsigned int Index, bool AdjustStack)
{
	this->Arrayptr[Index] = ArrayType();

	this->ArrayLength -= 1;
}