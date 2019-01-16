#pragma once

#include "Array.hpp"

template <typename ArrayType>
GS_CLASS SArray : public Array<ArrayType>
{
public:
	//Constructs a new SArray with N elements.
	SArray(unsigned int N);

	~SArray();

	//Places the Object after the last occupied element.
	void PopBack(const ArrayType & Object);

	//Removes the specified element.
	void RemoveElement(unsigned int Index);
};

template <typename ArrayType>
SArray<ArrayType>::SArray(unsigned int N)
{
	this->Arrayptr = new ArrayType[N];
}

template <typename ArrayType>
SArray<ArrayType>::~SArray()
{
	//Delete the heap allocated array located in Arrayptr.
	delete[] this->Arrayptr;
}

template <typename ArrayType>
void SArray<ArrayType>::PopBack(const ArrayType & Object)
{
	this->Arrayptr[this->LastIndex] = Object;

	this->LastIndex++;

	return;
}