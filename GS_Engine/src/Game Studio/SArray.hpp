#pragma once

#include "Array.hpp"

template <typename ArrayType>
GS_CLASS SArray : public Array<ArrayType>
{
public:
	//Constructs a new SArray with N elements.
	SArray(unsigned int N)
	{
		this->Arrayptr = new ArrayType[N];
	}

	~SArray()
	{
		//Delete the heap allocated array located in Arrayptr.
		delete[] this->Arrayptr;
	}

	//Places the Object after the last occupied element.
	void PopBack(const ArrayType & Object)
	{
		this->Arrayptr[this->LastIndex] = Object;

		this->LastIndex++;

		return;
	}

	void SetElement(unsigned int Index, const ArrayType & Object)
	{
		this->Arrayptr[Index] = Object;

		if (Index == this->LastIndex)
		{
			this->LastIndex = Index + 1;
		}

	return;
	}
};