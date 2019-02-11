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

	FVector() : Capacity(DEF_VEC_SIZE), Data(allocate(DEF_VEC_SIZE))
	{
	}

	explicit FVector(size_t length) : Capacity(length + EXTRA), Data(allocate(this->Capacity))
	{
	}

	explicit FVector(T Array[], size_t length) : Length(length), Capacity(length + EXTRA), Data(allocate(this->Capacity))
	{
		copyarray(Array, this->Data);
	}

	FVector(const FVector & Other) : Length(Other.Length), Capacity(Other.Capacity), Data(allocate(this->Capacity))
	{
		copyarray(Other.Data, this->Data);
	}

	FVector & operator=(T Other[])
	{
		const size_t length = (sizeof(*Other) / sizeof(T));

		if (length > this->Capacity)
		{
			Capacity = length + EXTRA;

			this->Data = allocate();
		}

		Length = length;

		copyarray(Other, this->Data, length);

		return *this;
	}

	FVector & operator=(const FVector & Other)
	{
		this->Capacity = Other.Capacity;
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
		checkfornew(1);

		++this->Length;

		for (size_t i = this->Length; i > index; i--)
		{
			this->Data[i] = this->Data[i - 1];
		}

		this->Data[index] = obj;
	}

	//Places the passed array at the specified index and shifts the rest of the array forward to fit it in.
	void insert(size_t index, T arr[], size_t length)
	{
		checkfornew(length);

		this->Length += length;

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
	T * allocate(size_t elementcount)
	{
		return new T[elementcount];
	}

	void copyarray(T* from, T* to)
	{
		for (size_t i = 0; i < this->Length; i++)
		{
			to[i] = from[i];
		}
	}

	void copyarray(T* from, T* to, size_t length)
	{
		for (size_t i = 0; i < length; i++)
		{
			to[i] = from[i];
		}
	}

	void checkfornew(size_t newelements)
	{
		if ((this->Length + newelements) > this->Capacity)
		{
			this->Capacity = this->Length * 2;

			T* buffer = allocate(this->Capacity);

			copyarray(this->Data, buffer);

			delete[] this->Data;

			this->Data = buffer;
		}
	}
};
