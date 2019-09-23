#pragma once

#include "Core.h"
#include <cstdlib>
#include <cstring>
#include <type_traits>

#define DEF_VEC_SIZE 15
#define EXTRA 5

template <typename T, typename LT = size_t>
class GS_EXPORT_ONLY FVector
{
private:
	LT Length = 0;
	LT Capacity = 0;

	T* Data = nullptr;

public:
	typedef T* iterator;
	typedef const T* const_iterator;

	//Constructs a new FVector.
	FVector()
	{
	}

	//Constructs a new FVector allocating space for the quantity of elements specified in length.
	explicit FVector(const size_t length) : Capacity(length), Data(allocate(this->Capacity))
	{
	}

	FVector(const size_t _Length, const T& _obj) : Capacity(_Length), Length(_Length), Data(allocate(this->Capacity))
	{
		for (size_t i = 0; i < this->Length; i++)
		{
			memcpy(&this->Data[i], &_obj, sizeof(T));
		}
	}

	FVector(const_iterator _start, const_iterator _end) : Capacity(_end - _start), Length(_end - _start), Data(allocate(this->Capacity))
	{
		copyarray(_start, this->Data);
	}

	//Constructs a new FVector filling the internal array with the contents of the passed in array.
	FVector(const size_t length, T Array[]) : Capacity(length), Length(length), Data(allocate(this->Capacity))
	{
		copyarray(Array, this->Data);
	}

	//Constructs a new FVector from another FVector.
	FVector(const FVector& Other) : Length(Other.Length), Capacity(Other.Capacity), Data(allocate(this->Capacity))
	{
		copyarray(Other.Data, this->Data);
	}

	//Assigns this object the data of the passed in FVector.
	FVector & operator=(const FVector & Other)
	{
		reallocIfExceeds(Other.Length - this->Length);
		copyarray(Other.Data, this->Data);

		return *this;
	}

	~FVector()
	{
		freearray();
	}

	[[nodiscard]] iterator begin() const
	{
		return &(this->Data[0]);
	}

	[[nodiscard]] iterator end() const
	{
		return &(this->Data[Length]);
	}

	T& front()
	{
		return this->Data[0];
	}

	[[nodiscard]] const T& front() const
	{
		return this->Data[0];
	}

	T& back()
	{
		return this->Data[this->Length];
	}

	[[nodiscard]] const T& back() const
	{
		return this->Data[this->Length];
	}

	void resize(const LT _Count)
	{
		this->Capacity = _Count;
		T* buffer = allocate(this->Capacity);
		copyarray(this->Data, buffer, this->Length);
		freearray();
		this->Data = buffer;
		return;
	}

	//Places the passed in element at the end of the array.
	void push_back(const T & obj)
	{
		reallocIfExceeds(1);
		copyarray(&obj, &this->Data[Length], 1);
		this->Length += 1;
	}

	//Places the passed in array at the end of the array.
	void push_back(const size_t length, T arr[])
	{
		reallocIfExceeds(length);
		copyarray(arr, &this->Data[this->Length]);
		this->Length += length;
	}

	//Places the passed in FVector at the end of the array.
	void push_back(const FVector& other)
	{
		reallocIfExceeds(other.Length);
		copyarray(other.Data, &this->Data[this->Length]);
		this->Length += other.Length;
	}

	template<typename... Args>
	void emplace_back(Args&&... _args)
	{
		reallocIfExceeds(1);

		new (this->Data + this->Length) T(std::forward<Args>(_args) ...);

		this->Length += 1;
	}

	//Deletes the array's last element.
	void pop_back()
	{
		this->Length -= 1;
	}

	//Places the passed in element at the specified index and shifts the rest of the array forward to fit it in.
	void insert(size_t index, const T & obj)
	{
		this->Length += 1;

		reallocIfExceeds(0);

		for (size_t i = this->Length; i > index; i--)
		{
			this->Data[i] = this->Data[i - 1];
		}

		this->Data[index] = obj;
	}

	//Places the passed array at the specified index and shifts the rest of the array forward to fit it in.
	void insert(const size_t index, T arr[], const size_t length)
	{
		this->Length += length;

		reallocIfExceeds(0);

		for (size_t i = this->Length; i > index; i--)
		{
			this->Data[i] = this->Data[i - length];
		}

		for (size_t i = 0; i < length; i++)
		{
			this->Data[index] = arr[i];
		}
	}

	//Overwrites existing data with the data from the passed array.
	void overlay(const size_t index, T arr[], const size_t length)
	{
		for (uint32 i = 0; i < length; ++i)
		{
			this->Data[index + i] = arr[i];
		}
	}

	//Adjusts the array's size to only fit the passed array and overwrites all existing data.
	void recreate(T arr[], const size_t length)
	{
		reallocIfExceeds(length - this->Length);

		this->Length = length;
		
		copyarray(arr, this->Data);

		return;
	}

	//Deletes the element at the specified index and shifts the array backwards to fill the empty space.
	void erase(const size_t index)
	{
		this->Length += 1;

		for (size_t i = index; i < this->Length; i++)
		{
			this->Data[i] = this->Data[i + 1];
		}
	}

	//Deletes all elements between index and index + length and shifts the entire array backwards to fill the empty space.
	void erase(const size_t index, const size_t length)
	{
		this->Length -= length;

		for (size_t i = index; i < this->Length; i++)
		{
			this->Data[i] = this->Data[i + length];
		}
	}

	//Looks for object inside of the array and when it finds it, it deletes it.
	void eraseObject(T & object)
	{
		for (size_t i = 0; i < this->Length; i++)
		{
			if (object == Data[i])
			{
				erase(i);

				break;
			}
		}

		this->Length -= 1;
	}

	iterator getElement(const size_t _i) { return &this->Data[_i]; }

	//Returns the element at the specified index. ONLY CHECKS FOR OUT OF BOUNDS IN DEBUG BUILDS.
	INLINE T& operator[](const size_t index)
	{
		#ifdef GS_DEBUG
		if (index > this->Capacity)
		{
			throw("Out of bounds!");
		}
		#endif

		return this->Data[index];
	}

	//Returns the element at the specified index. ONLY CHECKS FOR OUT OF BOUNDS IN DEBUG BUILDS.
	INLINE const T& operator[](const size_t index) const
	{
		#ifdef GS_DEBUG
		if (index > this->Capacity)
		{
			throw "Out of bounds!";
		}
		#endif

		return this->Data[index];
	}

	//Returns the occupied elements count.
	INLINE size_t length() const
	{
		return this->Length;
	}

	//Returns the total allocated elements count. 
	INLINE size_t capacity() const { return this->Capacity; }

	//Returns a pointer to the allocated array.
	INLINE T* data()
	{
		return this->Data;
	}

	//Returns a pointer to the allocated array.
	INLINE const T* data() const
	{
		return this->Data;
	}

private:
	//Allocates a new a array of type T with enough space to hold elementcount elements.
	T* allocate(const size_t elementcount)
	{
		return SCAST(T*, malloc(elementcount * sizeof(T)));
	}

	void freearray()
	{
		free(this->Data);
		this->Data = nullptr;
	}

	//Fills array to with from.
	void copyarray(T * from, T * to)
	{
		memcpy(to, from, this->Capacity * sizeof(T));
	}

	//Fills array to with from.
	void copyarray(const T* from, T* to)
	{
		memcpy(to, from, this->Capacity * sizeof(T));
	}

	//Fills array to with from.
	void copyarray(T* from, T* to, size_t _ElementCount)
	{
		memcpy(to, from, _ElementCount * sizeof(T));
	}

	//Fills array to with from.
	void copyarray(const T* from, T* to, size_t _ElementCount)
	{
		memcpy(to, from, _ElementCount * sizeof(T));
	}

	//Allocates a new array if Length + newelements exceeds the allocated space.
	void reallocIfExceeds(const size_t additionalelements)
	{
		if (this->Length + additionalelements > this->Capacity)
		{
			this->Capacity = (this->Length * 2) + additionalelements;

			T* buffer = allocate(this->Capacity);

			copyarray(this->Data, buffer);

			freearray();

			this->Data = buffer;
		}
	}
};