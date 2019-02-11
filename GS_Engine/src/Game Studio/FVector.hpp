#pragma once

#include "Core.h"

#define DEF_VEC_SIZE 15
#define EXTRA 5

template <typename T>
GS_CLASS FVector
{
private:
	size_t Length = 0;
	size_t Capacity = 0;

	T * Data = nullptr;

public:

	//Constructs a new FVector and allocates some previsional space.
	FVector() : Capacity(DEF_VEC_SIZE), Data(allocate(DEF_VEC_SIZE))
	{
	}

	//Constructs a new FVector allocating space for the quantity of elements specified in length.
	explicit FVector(size_t length) : Capacity(length + EXTRA), Data(allocate(this->Capacity))
	{
	}

	//Constructs a new FVector filling the internal array with the contents of the passed in array.
	explicit FVector(T Array[], size_t length) : Length(length), Capacity(length + EXTRA), Data(allocate(this->Capacity))
	{
		copyarray(Array, this->Data);
	}

	//Constructs a new FVector from another FVector.
	FVector(const FVector & Other) : Length(Other.Length), Capacity(Other.Capacity), Data(allocate(this->Capacity))
	{
		copyarray(Other.Data, this->Data);
	}

	//Assigns the internal array the contents of the passed in array.
	FVector & operator=(T Other[])
	{
		const size_t length = (sizeof(*Other) / sizeof(T));

		checkfornew(length - this->Length);

		Length = length;

		copyarray(Other, this->Data);

		return *this;
	}

	//Assigns this object the data of the passed in FVector.
	FVector & operator=(const FVector & Other)
	{
		checkfornew(Other.Length - this->Length);

		this->Length = Other.Length;

		copyarray(Other.Data, this->Data);

		return *this;
	}

	~FVector()
	{
		delete this->Data;
	}

	//Places the passed in element at the end of the array.
	void push_back(const T & obj)
	{
		checkfornew(1);

		this->Data[Length] = obj;

		++this->Length;
	}

	//Deletes the array's last element.
	void pop_back()
	{
		--this->Length;
	}

	//Places the passed in element at the specified index and shifts the rest of the array forward to fit it in.
	void insert(size_t index, const T & obj)
	{
		++this->Length;

		checkfornew();

		for (size_t i = this->Length; i > index; i--)
		{
			this->Data[i] = this->Data[i - 1];
		}

		this->Data[index] = obj;
	}

	//Places the passed array at the specified index and shifts the rest of the array forward to fit it in.
	void insert(size_t index, T arr[], size_t length)
	{
		this->Length += length;

		checkfornew();

		for (size_t i = this->Length; i > index; i--)
		{
			this->Data[i] = this->Data[i - length];
		}

		for (size_t i = 0; i < length; i++)
		{
			this->Data[index] = arr[i];
		}
	}

	//Overwrites existing data with the data from tha passed array.
	void overlay(size_t index, T arr[], size_t length)
	{
		for (uint32 i = 0; i < length; ++i)
		{
			this->Data[index + i] = arr[i];
		}
	}

	//Deletes the element at the specified index and shifts the array backwards to fill the empty space.
	void erase(size_t index)
	{
		--this->Length;

		for (size_t i = index; i < this->Length; i++)
		{
			this->Data[i] = this->Data[i + 1];
		}
	}

	//Deletes all elements between index and index + length and shifts the entiry array backwards to fill the empty space.
	void erase(size_t index, size_t length)
	{
		this->Length -= length;

		for (size_t i = index; i < this->Length; i++)
		{
			this->Data[i] = this->Data[i + length];
		}
	}

	//Returns the element at the specified index. ONLY CHECKS FOR OUT OF BOUNDS IN DEBUG BUILDS.
	T & operator[](size_t index)
	{
#ifdef GS_DEBUG
		if (index > this->Length)
		{
			throw("Out of bounds!");
		}
#endif

		return this->Data[index];
	}

	//Retuns the ocuppied elements count.
	size_t length() const
	{
		return this->Length;
	}

	//Returns a pointer to the allocated array.
	T * data()
	{
		return this->Data;
	}

private:
	//Allocates a new a array of type T with enough space to hold elementcount elements.
	T * allocate(size_t elementcount)
	{
		return new T[elementcount];
	}

	//Fills array to with from.
	void copyarray(T* from, T* to)
	{
		for (size_t i = 0; i < this->Length; i++)
		{
			to[i] = from[i];
		}
	}

	//Allocates a new array if Length + newelements exceeds the allocated space.
	void checkfornew()
	{
		if (this->Length > this->Capacity)
		{
			this->Capacity = this->Length * 2;

			T * buffer = allocate(this->Capacity);

			copyarray(this->Data, buffer);

			delete[] this->Data;

			this->Data = buffer;
		}
	}

	void checkfornew(size_t additionalelements)
	{
		if (this->Length + additionalelements > this->Capacity)
		{
			this->Capacity = (this->Length * 2) + additionalelements;

			T * buffer = allocate(this->Capacity);

			copyarray(this->Data, buffer);

			delete[] this->Data;

			this->Data = buffer;
		}
	}
};
