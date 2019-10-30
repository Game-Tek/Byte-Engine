#pragma once

#include "Core.h"
#include <cstdlib>
#include <cstring>
#include <type_traits>
#include <assert.h>
#include "Resources/Stream.h"

template <typename T, typename LT = size_t>
class FVector
{
	LT Capacity = 0;
	LT Length = 0;

	T* Data = nullptr;

	//Copies data from _from to _to only if this->Data is not nullptr.
	void copyArray(const T* _from, T* _to, size_t _ElementCount)
	{
		memcpy(_to, _from, _ElementCount * sizeof(T));
	}

	//Allocates a new a array of type T with enough space to hold elementcount elements.
	static T* allocate(const size_t _ElementCount)
	{
		return SCAST(T*, malloc(_ElementCount * sizeof(T)));
	}

	//Deletes this->Data only if it in not nullptr.
	//Set this->Data as nullptr.
	void freeData()
	{
		free(this->Data);
		this->Data = nullptr;
	}

	//Allocates a new array if Length + newelements exceeds the allocated space.
	void reallocIfExceeds(const int64 _AdditionalElements)
	{
		if (this->Length + _AdditionalElements > this->Capacity)
		{
			const size_t newCapacity = (this->Length * 2) + _AdditionalElements;
			T* newData = allocate(newCapacity);
			copyArray(this->Data, newData, this->Capacity);
			freeData();
			this->Capacity = newCapacity;
			this->Data = newData;
		}
	}
public:
	typedef T* iterator;
	typedef const T* const_iterator;

	friend OutStream& operator<<(OutStream& _Archive, FVector<T>& _FV)
	{
		_Archive.Write(_FV.Capacity);
		_Archive.Write(_FV.Length);

		_Archive.Write(_FV.Capacity, _FV.Data);

		return _Archive;
	}

	friend InStream& operator>>(InStream& _Archive, FVector<T>& _FV)
	{
		size_t new_capacity = 0, new_length = 0;
		_Archive.Read(&new_capacity);
		_Archive.Read(&new_length);

		_FV.reallocIfExceeds(new_length);

		_Archive.Read(new_capacity, _FV.Data);

		_FV.Length = new_length;

		return _Archive;
	}

	//Constructs a new FVector.
	FVector() = delete;

	//Constructs a new FVector allocating space for the quantity of elements specified in length.
	explicit FVector(const size_t _Capacity) : Capacity(_Capacity), Length(0), Data(allocate(this->Capacity))
	{
	}

	FVector(const size_t _Length, const T& _Obj) : Capacity(_Length), Length(_Length), Data(allocate(this->Capacity))
	{
		for (size_t i = 0; i < this->Length; ++i)
		{
			copyArray(&_Obj, getElement(i), 1);
		}
	}

	FVector(const_iterator _Start, const_iterator _End) : Capacity(_End - _Start), Length(_End - _Start), Data(allocate(this->Capacity))
	{
		copyArray(_Start, this->Data);
	}

	//Constructs a new FVector filling the internal array with the contents of the passed in array.
	FVector(const size_t _Length, T _Array[]) : Capacity(_Length), Length(_Length), Data(allocate(this->Capacity))
	{
		copyArray(_Array, this->Data, this->Length);
	}

	//Constructs a new FVector from another FVector.
	FVector(const FVector& _Other) : Capacity(_Other.Capacity), Length(_Other.Length), Data(allocate(this->Capacity))
	{
		copyArray(_Other.Data, this->Data, this->Length);
	}

	//Assigns this object the data of the passed in FVector.
	FVector& operator=(const FVector& _Other)
	{
		reallocIfExceeds(_Other.Length - this->Length);
		copyArray(_Other.Data, this->Data, _Other.Length);
		this->Length = _Other.Length;
		return *this;
	}

	~FVector()
	{
		assert(this->Data);
		freeData();
	}

	[[nodiscard]] iterator begin() { return this->Data; }

	[[nodiscard]] iterator end() { return &this->Data[Length]; }

	[[nodiscard]] const_iterator begin() const { return this->Data; }

	[[nodiscard]] const_iterator end() const { return &this->Data[this->Length]; }

	T& front() { return this->Data[0]; }

	T& back() {	return this->Data[this->Length]; }

	[[nodiscard]] const T& front() const { return this->Data[0]; }

	[[nodiscard]] const T& back() const { return this->Data[this->Length]; }

	void resize(const LT _Count)
	{
		reallocIfExceeds(_Count - this->Length);
		this->Length = _Count;
		return;
	}

	void shrink(const LT _Count)
	{
		this->Capacity = _Count;
		this->Length = _Count;
		T* buffer = allocate(this->Capacity);
		copyArray(this->Data, buffer, this->Length);
		freeData();
		this->Data = buffer;
		return;
	}

	//Places the passed in element at the end of the array.
	void push_back(const T& _Obj)
	{
		reallocIfExceeds(1);
		copyArray(&_Obj, getElement(this->Length), 1);
		this->Length += 1;
	}

	//Places the passed in array at the end of the array.
	void push_back(const size_t _Length, const T _Arr[])
	{
		reallocIfExceeds(_Length);
		copyArray(_Arr, getElement(this->Length), _Length);
		this->Length += _Length;
	}

	//Places the passed in FVector at the end of the array.
	void push_back(const FVector& _Other)
	{
		reallocIfExceeds(this->Length - _Other.Length);
		copyArray(_Other.Data, getElement(this->Length), _Other.Length);
		this->Length += _Other.Length;
	}

	template<typename... Args>
	void emplace_back(Args&&... _Args)
	{
		reallocIfExceeds(1);
		new (this->Data + this->Length) T(std::forward<Args>(_Args) ...);
		this->Length += 1;
	}

	//Deletes the array's last element.
	void pop_back()
	{
		if (this->Length != 0)
		{
			this->Length -= 1;
		}
	}

	//Places the passed in element at the specified index and shifts the rest of the array forward to fit it in.
	void insert(size_t _Index, const T& _Obj)
	{
		reallocIfExceeds(1);
		copyArray(getElement(_Index), getElement(_Index + 1), this->Length - _Index);
		copyArray(&_Obj, getElement(_Index), 1);
		this->Length += 1;
	}

	//Places the passed array at the specified index and shifts the rest of the array forward to fit it in.
	void insert(const size_t _Length, T _Arr[], const size_t _Index)
	{
		reallocIfExceeds(_Length);
		copyArray(getElement(_Index), getElement(_Index + _Length), this->Length - _Index);
		copyArray(_Arr, getElement(_Index), _Length);
		this->Length += _Length;
	}

	//Overwrites existing data with the data from the passed array.
	void overwrite(const size_t _Length, T _Arr[], const size_t _Index)
	{
		reallocIfExceeds((this->Length - _Length) + _Index);
		copyArray(_Arr, getElement(_Index), _Length);
		this->Length += (this->Length - _Length) + _Index;
	}

	//Adjusts the array's size to only fit the passed array and overwrites all existing data.
	void recreate(const size_t _Length, T _Arr[])
	{
		reallocIfExceeds(_Length - this->Length);
		copyArray(_Arr, this->Data, _Length);
		this->Length = _Length;
		return;
	}

	//Deletes the element at the specified index and shifts the array backwards to fill the empty space.
	void erase(const size_t _Index)
	{
		copyArray(getElement(_Index + 1), getElement(_Index), this->Length - _Index);
		this->Length -= 1;
	}

	//Deletes all elements between index and index + length and shifts the entire array backwards to fill the empty space.
	void erase(const size_t _Index, const size_t _Length)
	{
		copyArray(getElement(_Index + _Length), getElement(_Index), this->Length - _Index);
		this->Length -= _Length;
	}

	size_t find(const T& _Obj)
	{
		for (size_t i = 0; i < this->Length; i++)
		{
			if (_Obj == Data[i])
			{
				return i;
			}
		}
		return ~0ULL;
	}

	//Looks for object inside of the array and when it finds it, it deletes it.
	void eraseObject(T & _Obj)
	{
		auto res = find(_Obj);
		if(res != ~0ULL)
		{
			erase(res);
			this->Length -= 1;
		}
	}

	iterator getElement(const size_t _I) { return &this->Data[_I]; }

	//Returns the element at the specified index. ONLY CHECKS FOR OUT OF BOUNDS IN DEBUG BUILDS.
	INLINE T& operator[](const size_t index)
	{
		#ifdef GS_DEBUG
		if (index > this->Capacity)
		{
			throw "Out of bounds!";
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
			throw "Entered index is not accessible, array is not as large.";
		}
		#endif

		return this->Data[index];
	}

	//Returns the occupied elements count.
	INLINE size_t length() const { return this->Length; }

	//Returns the total allocated elements count. 
	INLINE size_t capacity() const { return this->Capacity; }

	//Returns a pointer to the allocated array.
	INLINE T* data() { return this->Data; }

	//Returns a pointer to the allocated array.
	INLINE const T* data() const { return this->Data; }
};
