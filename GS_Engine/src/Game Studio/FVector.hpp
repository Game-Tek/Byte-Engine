#pragma once

#include "Core.h"

#define DEF_VEC_SIZE 15

template <typename T>
GS_CLASS FVector
{
private:
	size_t Length = 0;
	size_t Capacity = 0;

	T * Data = nullptr;

public:

	FVector() : Capacity(DEF_VEC_SIZE)
	{
		this->Data = allocate(DEF_VEC_SIZE);
	}

	FVector(size_t length) : Capacity(length)
	{
		this->Data = allocate(length);
	}

	FVector(const FVector & Other) : Length(Other.Length), Capacity(Other.Capacity)
	{
		Data = allocate(this->Capacity);

		copyarray(Other.Data, this->Data);
	}

	void push_back(const T & obj)
	{
		checkfornew(1);

		this->Data[Length] = obj;

		this->Length++;
	}

	void pop_back()
	{
		this->Length--;
	}

	void place(size_t index, const T & obj)
	{
		checkfornew(1);

		this->Length++;

		for (size_t i = this->Length; i > this->index; i--)
		{
			this->Data[i] = this->Data[i - 1];
		}

		this->Data[index] = obj;
	}

	void place(size_t index, T arr[], size_t length)
	{
		checkfornew(length);

		this->Length += length;

		for (size_t i = this->Length; i > this->index; i--)
		{
			this->Data[i] = this->Data[i - length];
		}

		for (size_t i = 0; i < length; i++)
		{
			this->Data[index] = arr[i];
		}
	}

	void erase(size_t index)
	{
		this->Length--;

		for (size_t i = index; i < this->Length; i++)
		{
			this->Data[i] = this->Data[i + 1];
		}
	}

	void erase(size_t index, size_t length)
	{
		this->Length -= length;

		for (size_t i = index; i < this->Length; i++)
		{
			this->Data[i] = this->Data[i + length];
		}
	}

	T & operator[] (size_t index)
	{
		return this->Data[index];
	}

	size_t size() const
	{
		return this->Length;
	}

private:
	T * allocate(size_t elementcount)
	{
		return new T[elementcount];
	}

	void copyarray(T * from, T * to)
	{
		for (size_t i = 0; i < this->Length; i++)
		{
			to[i] = from[i];
		};

		return;
	}

	void checkfornew(size_t newelements)
	{
		if ((this->Length + newelements) > this->Capacity)
		{
			this->Capacity = this->Length * 2;

			T * buffer = allocate(this->Capacity);

			copyarray(this->Data, buffer);

			delete[] this->Data;

			this->Data = buffer;
		}

		return;
	}
};