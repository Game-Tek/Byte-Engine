#pragma once

#include "Array.hpp"

template <typename ArrayType>
GS_CLASS DArray : public Array<ArrayType>
{
public:
	DArray()
	{
		unsigned char Size = 5 + this->DEFAULT_ARRAY_SIZE;

		//Allocate a new array and save a pointer to it inside Arrayptr.
		this->Arrayptr = AllocateNewArray(Size);

		//Set the array's capacity as Size.
		this->ArrayCapacity = Size;
	}

	//Constructs a DArray with N elements.
	DArray(unsigned int N)
	{
		//Determine which size the initial array will be.
		unsigned int Size = DEFAULT_ARRAY_SIZE + ((N < this->DEFAULT_ARRAY_SIZE) ? this->DEFAULT_ARRAY_SIZE : N);

		//Allocate a new array and save a pointer to it inside Arrayptr.
		(*this->Arrayptr) = AllocateNewArray(Size);

		//Set the array's capacity as Size.
		this->ArrayCapacity = Size;
	}

	~DArray()
	{
		for (unsigned int i = 0; i < this->ArrayLength; i++)
		{
			delete this->Arrayptr[i];
		}

		//Delete the heap allocated array located in Arrayptr.
		delete[] this->Arrayptr;
	}

	//inserts the object after the last occupied element.
	void PopBack(const ArrayType & Object)
	{
		//Check if adding a new element will exceed the allocated elements.
		if (this->LastIndex + 1 > this->ArrayCapacity)
		{
			//Determine the size of the new array.
			unsigned int NewSize = this->ArrayLength + 1 + this->DEFAULT_ARRAY_SIZE;

			//Allocate a new array to a temp pointer.
			ArrayType * NewArray = AllocateNewArray(NewSize);

			//Fill the new array with the contents of the old/current one.
			FillArray(NewArray, (**this->Arrayptr));

			//Delete the old array which Arrayptr is pointing to.
			delete[] this->Arrayptr;

			//Set the Array pointer to the recently created and filled array.
			(*this->Arrayptr) = NewArray;

			//Update the total number of elements count.
			this->ArrayCapacity = NewSize;
		}

		//Fill the last element with Element.
		this->Arrayptr[this->LastIndex + 1] = new ArrayType(Object);

		//We update the number of elements count.
		this->ArrayLength++;
		this->LastIndex++;

		return;
	}

	//Removes the specified element.
	void RemoveElement(int Index)
	{
		delete this->Arrayptr[Index];
		this->Arrayptr[Index] = nullptr;

		return;
	}

private:
	const int DEFAULT_ARRAY_SIZE = 5;

	//Allocates/creteas a new array and returns a pointer to it.
	ArrayType * AllocateNewArray(int N)
	{
		//Return a new heap allocated array of N size.
		return new ArrayType[N];
	}

	//Fills an array with the contents of another.
	void FillArray(const ArrayType & ArrayToFill, const ArrayType & SourceArray)
	{
		//Fill ArrayToFill with the elements of SourceArray.
		for (unsigned short i = 0; i < this->ArrayLength; i++)
		{
			ArrayToFill[i] = SourceArray[i];
		}

		return;
	}
};