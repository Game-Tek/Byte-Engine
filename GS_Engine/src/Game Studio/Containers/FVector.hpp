#pragma once

#include "Core.h"
#include <cstdlib>
#include <cstring>
#include <type_traits>
#include <initializer_list>
#include "Pair.h"

template <typename T>
class FVector
{
	uint32 capacity = 0;
	uint32 length = 0;

	T* data = nullptr;

	/**
	 * \brief Copies data from _from to _to.
	 * \param _from Pointer from where to grab the data.
	 * \param _to Pointer to write the data to.
	 * \param _ElementCount How many elements of this vector's T to write to _to.
	 */
	static void copyArray(const T* _from, T* _to, const size_t _ElementCount)
	{
		memcpy(_to, _from, _ElementCount * sizeof(T));
	}

	/**
	 * \brief Allocates a new a array of type T with enough space to hold _ElementCount elements.
	 * \param _ElementCount How many elements of this vector's T to allocate space for.
	 * \return T pointer to the newly allocated memory.
	 */
	static T* allocate(const size_t _ElementCount)
	{
		return SCAST(T*, malloc(_ElementCount * sizeof(T)));
	}

	/**
	 * \brief Deletes the data found at this vector's data and sets data as nullptr.
	 */
	void freeData()
	{
		if (this->data)
		{
			free(this->data);
		}

		this->data = nullptr;
	}

	/**
	 * \brief Reallocates this->data to a new array if (this->length + _AdditionalElements) exceeds the allocated space.\n
	 * Also deletes the data found at the old data.
	 * Growth of the array is geometric.
	 * \param _AdditionalElements How many elements of this vector's T are you trying to check if fit in the already allocated array.\n
	 * Number can be negative.
	 */
	void reallocIfExceeds(const int_64 _AdditionalElements)
	{
		if (this->length + _AdditionalElements > this->capacity)
		{
			const size_t newCapacity = (this->length * 2) + _AdditionalElements;
			T* newData = allocate(newCapacity);
			copyArray(this->data, newData, this->capacity);
			freeData();
			this->capacity = newCapacity;
			this->data = newData;
		}
	}

public:
	typedef T* iterator;
	typedef const T* const_iterator;
	typedef uint32 length_type;

	//friend OutStream& operator<<(OutStream& _Archive, FVector<T>& _FV)
	//{
	//	_Archive.Write(_FV.capacity);
	//	_Archive.Write(_FV.length);
	//
	//	for (uint32 i = 0; i < _FV.length; ++i)
	//	{
	//		_Archive << _FV.data[i];
	//	}
	//
	//	return _Archive;
	//}
	//
	//friend InStream& operator>>(InStream& _Archive, FVector<T>& _FV)
	//{
	//	size_t new_capacity = 0, new_length = 0;
	//	_Archive.Read(&new_capacity);
	//	_Archive.Read(&new_length);
	//
	//	_FV.reallocIfExceeds(new_length);
	//
	//	//_Archive.Read(new_capacity, _FV.data);
	//
	//	for (uint32 i = 0; i < new_length; ++i)
	//	{
	//		_Archive >> _FV.data[i];
	//	}
	//
	//	_FV.length = new_length;
	//
	//	return _Archive;
	//}

	//Constructs a new FVector.
	FVector() : capacity(10), length(0), data(allocate(this->capacity))
	{
	}

	//Constructs a new FVector allocating space for the quantity of elements specified in length.
	explicit FVector(const size_t _Capacity) : capacity(_Capacity), length(0), data(allocate(this->capacity))
	{
	}

	explicit FVector(const size_t _Capacity, const size_t length) : capacity(_Capacity), length(length),
	                                                                data(allocate(this->capacity))
	{
	}

	FVector(const size_t _Length, const T& _Obj) : capacity(_Length), length(_Length), data(allocate(this->capacity))
	{
		for (size_t i = 0; i < this->length; ++i)
		{
			copyArray(&_Obj, getElement(i), 1);
		}
	}

	FVector(const std::initializer_list<T>& _InitializerList) :
		capacity(_InitializerList.end() - _InitializerList.begin()), length(this->capacity),
		data(allocate(this->capacity))
	{
		copyArray(_InitializerList.begin(), this->data, this->length);
	}

	FVector(const_iterator _Start, const_iterator _End) : capacity(_End - _Start), length(_End - _Start),
	                                                      data(allocate(this->capacity))
	{
		copyArray(_Start, this->data);
	}

	//Constructs a new FVector filling the internal array with the contents of the passed in array.
	FVector(const size_t _Length, T _Array[]) : capacity(_Length), length(_Length), data(allocate(this->capacity))
	{
		copyArray(_Array, this->data, this->length);
	}

	//Constructs a new FVector from another FVector.
	FVector(const FVector& _Other) : capacity(_Other.capacity), length(_Other.length), data(allocate(this->capacity))
	{
		copyArray(_Other.data, this->data, this->length);
	}

	//Assigns this object the data of the passed in FVector.
	FVector& operator=(const FVector& _Other)
	{
		reallocIfExceeds(_Other.length - this->length);
		copyArray(_Other.data, this->data, _Other.length);
		this->length = _Other.length;
		return *this;
	}

	~FVector()
	{
		for(auto begin = this->begin(); begin != this->end(); ++begin)
		{
			begin->~T();
		}
		
		freeData();
	}

	[[nodiscard]] iterator begin() { return this->data; }

	[[nodiscard]] iterator end() { return &this->data[this->length]; }

	[[nodiscard]] const_iterator begin() const { return this->data; }

	[[nodiscard]] const_iterator end() const { return &this->data[this->length]; }

	T& front() { return this->data[0]; }

	T& back() { return this->data[this->length]; }

	[[nodiscard]] const T& front() const { return this->data[0]; }

	[[nodiscard]] const T& back() const { return this->data[this->length]; }

	void resize(const uint32 _Count)
	{
		reallocIfExceeds(_Count - this->length);
		this->length = _Count;
		return;
	}

	void forceRealloc(const uint32 count)
	{
		this->data = allocate(count);
		this->capacity = count;
		this->length = 0;
		return;
	}

	void shrink(const uint32 _Count)
	{
		this->capacity = _Count;
		this->length = _Count;
		T* buffer = allocate(this->capacity);
		copyArray(this->data, buffer, this->length);
		freeData();
		this->data = buffer;
		return;
	}

	//Places the passed in element at the end of the array.
	void push_back(const T& _Obj)
	{
		reallocIfExceeds(1);
		//copyArray(&_Obj, getElement(this->length), 1);
		::new(this->data + this->length) T(_Obj);
		this->length += 1;
	}

	//Places the passed in array at the end of the array.
	void push_back(const size_t _Length, const T _Arr[])
	{
		reallocIfExceeds(_Length);
		copyArray(_Arr, getElement(this->length), _Length);
		this->length += _Length;
	}

	//Places the passed in FVector at the end of the array.
	void push_back(const FVector& _Other)
	{
		reallocIfExceeds(this->length - _Other.length);
		copyArray(_Other.data, getElement(this->length), _Other.length);
		this->length += _Other.length;
	}

	template <typename... Args>
	uint32 emplace_back(Args&&... _Args)
	{
		reallocIfExceeds(1);
		::new(this->data + this->length) T(std::forward<Args>(_Args) ...);
		return this->length += 1;
	}

	//Deletes the array's last element.
	void pop_back()
	{
		if (this->length != 0)
		{
			this->length -= 1;
		}
	}

	//Places the passed in element at the specified index and shifts the rest of the array forward to fit it in.
	uint32 push(size_t _Index, const T& _Obj)
	{
		reallocIfExceeds(1);
		copyArray(getElement(_Index), getElement(_Index + 1), this->length - _Index);
		::new(this->data + this->length) T(_Obj);
		return this->length += 1;
	}

	//Places the passed array at the specified index and shifts the rest of the array forward to fit it in.
	void push(const size_t _Length, T _Arr[], const size_t _Index)
	{
		reallocIfExceeds(_Length);
		copyArray(getElement(_Index), getElement(_Index + _Length), this->length - _Index);
		copyArray(_Arr, getElement(_Index), _Length);
		this->length += _Length;
	}

	//Overwrites existing data with the data from the passed array.
	void overwrite(const size_t _Length, T _Arr[], const size_t _Index)
	{
		reallocIfExceeds((this->length - _Length) + _Index);
		copyArray(_Arr, getElement(_Index), _Length);
		this->length += (this->length - _Length) + _Index;
	}

	//Adjusts the array's size to only fit the passed array and overwrites all existing data.
	void recreate(const size_t _Length, T _Arr[])
	{
		reallocIfExceeds(_Length - this->length);
		copyArray(_Arr, this->data, _Length);
		this->length = _Length;
		return;
	}

	//Deletes the element at the specified index and shifts the array backwards to fill the empty space.
	void pop(const size_t _Index)
	{
		copyArray(getElement(_Index + 1), getElement(_Index), this->length - _Index);
		this->length -= 1;
	}

	//Deletes all elements between index and index + length and shifts the entire array backwards to fill the empty space.
	void popRange(const size_t _Index, const size_t _Length)
	{
		copyArray(getElement(_Index + _Length), getElement(_Index), this->length - _Index);
		this->length -= _Length;
	}

	iterator find(const T& _Obj)
	{
		for (size_t i = 0; i < this->length; i++)
		{
			if (_Obj == data[i])
			{
				return getElement(i);
			}
		}

		return this->end();
	}

	//Looks for object inside of the array and when it finds it, it deletes it.
	void eraseObject(T& _Obj)
	{
		auto res = find(_Obj);
		if (res != this->end())
		{
			pop(res);
			this->length -= 1;
		}
	}

	/**
	 * \brief Returns an iterator to an specified index. DOES NOT CHECK FOR BOUNDS, but underlying getter does, only in debug builds.
	 * \param _I Index to the element to be retrieved.
	 * \return iterator to the element at _I.
	 */
	iterator getElement(const size_t _I) { return &this->data[_I]; }

	//Returns the element at the specified index. ONLY CHECKS FOR OUT OF BOUNDS IN DEBUG BUILDS.
	INLINE T& operator[](const size_t index)
	{
#ifdef GS_DEBUG
		if (index > this->capacity)
		{
			throw "Out of bounds!";
		}
#endif

		return this->data[index];
	}

	//Returns the element at the specified index. ONLY CHECKS FOR OUT OF BOUNDS IN DEBUG BUILDS.
	INLINE const T& operator[](const size_t index) const
	{
#ifdef GS_DEBUG
		if (index > this->capacity)
		{
			throw "Entered index is not accessible, array is not as large.";
		}
#endif

		return this->data[index];
	}

	//Returns the occupied elements count.
	INLINE size_t getLength() const { return this->length; }

	//Returns the total allocated elements count. 
	INLINE size_t getCapacity() const { return this->capacity; }

	//Returns a pointer to the allocated array.
	INLINE T* getData() { return this->data; }

	//Returns a pointer to the allocated array.
	INLINE const T* getData() const { return this->data; }
};
