#pragma once

#include "Core.h"
#include <cstdlib>
#include <cstring>
#include <type_traits>
#include <initializer_list>
#include "Resources/Stream.h"
#include "Pair.h"

template <typename _T, typename _LT = size_t>
class FVector
{
	_LT capacity = 0;
	_LT length = 0;

	_T* data = nullptr;

	/**
	 * \brief Copies data from _from to _to.
	 * \param _from Pointer from where to grab the data.
	 * \param _to Pointer to write the data to.
	 * \param _ElementCount How many elements of this vector's _T to write to _to.
	 */
	static void copyArray(const _T* _from, _T* _to, const size_t _ElementCount)
	{
		memcpy(_to, _from, _ElementCount * sizeof(_T));
	}

	/**
	 * \brief Allocates a new a array of type _T with enough space to hold _ElementCount elements.
	 * \param _ElementCount How many elements of this vector's _T to allocate space for.
	 * \return _T pointer to the newly allocated memory.
	 */
	static _T* allocate(const size_t _ElementCount)
	{
		return SCAST(_T*, malloc(_ElementCount * sizeof(_T)));
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
	 * \param _AdditionalElements How many elements of this vector's _T are you trying to check if fit in the already allocated array.\n
	 * Number can be negative.
	 */
	void reallocIfExceeds(const int_64 _AdditionalElements)
	{
		if (this->length + _AdditionalElements > this->capacity)
		{
			const size_t newCapacity = (this->length * 2) + _AdditionalElements;
			_T* newData = allocate(newCapacity);
			copyArray(this->data, newData, this->capacity);
			freeData();
			this->capacity = newCapacity;
			this->data = newData;
		}
	}
public:
	typedef _T* iterator;
	typedef const _T* const_iterator;

	//friend OutStream& operator<<(OutStream& _Archive, FVector<_T>& _FV)
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
	//friend InStream& operator>>(InStream& _Archive, FVector<_T>& _FV)
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

	FVector(const size_t _Length, const _T& _Obj) : capacity(_Length), length(_Length), data(allocate(this->capacity))
	{
		for (size_t i = 0; i < this->length; ++i)
		{
			copyArray(&_Obj, getElement(i), 1);
		}
	}

	FVector(const std::initializer_list<_T>& _InitializerList) : capacity(_InitializerList.end() - _InitializerList.begin()), length(this->capacity), data(allocate(this->capacity))
	{
		copyArray(_InitializerList.begin(), this->data, this->length);
	}
	
	FVector(const_iterator _Start, const_iterator _End) : capacity(_End - _Start), length(_End - _Start), data(allocate(this->capacity))
	{
		copyArray(_Start, this->data);
	}

	//Constructs a new FVector filling the internal array with the contents of the passed in array.
	FVector(const size_t _Length, _T _Array[]) : capacity(_Length), length(_Length), data(allocate(this->capacity))
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
		freeData();
	}

	[[nodiscard]] iterator begin() { return this->data; }

	[[nodiscard]] iterator end() { return &this->data[this->length]; }

	[[nodiscard]] const_iterator begin() const { return this->data; }

	[[nodiscard]] const_iterator end() const { return &this->data[this->length]; }

	_T& front() { return this->data[0]; }

	_T& back() { return this->data[this->length]; }

	[[nodiscard]] const _T& front() const { return this->data[0]; }

	[[nodiscard]] const _T& back() const { return this->data[this->length]; }

	void resize(const _LT _Count)
	{
		reallocIfExceeds(_Count - this->length);
		this->length = _Count;
		return;
	}

	void forceRealloc(const _LT count)
	{
		this->data = allocate(count);
		this->capacity = count;
		this->length = 0;
		return;
	}
	
	void shrink(const _LT _Count)
	{
		this->capacity = _Count;
		this->length = _Count;
		_T* buffer = allocate(this->capacity);
		copyArray(this->data, buffer, this->length);
		freeData();
		this->data = buffer;
		return;
	}

	//Places the passed in element at the end of the array.
	void push_back(const _T& _Obj)
	{
		reallocIfExceeds(1);
		//copyArray(&_Obj, getElement(this->length), 1);
		::new (this->data + this->length) _T(_Obj);
		this->length += 1;
	}

	//Places the passed in array at the end of the array.
	void push_back(const size_t _Length, const _T _Arr[])
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

	template<typename... Args>
	void emplace_back(Args&&... _Args)
	{
		reallocIfExceeds(1);
		::new (this->data + this->length) _T(std::forward<Args>(_Args) ...);
		this->length += 1;
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
	void push(size_t _Index, const _T& _Obj)
	{
		reallocIfExceeds(1);
		copyArray(getElement(_Index), getElement(_Index + 1), this->length - _Index);
		copyArray(&_Obj, getElement(_Index), 1);
		this->length += 1;
	}

	//Places the passed array at the specified index and shifts the rest of the array forward to fit it in.
	void push(const size_t _Length, _T _Arr[], const size_t _Index)
	{
		reallocIfExceeds(_Length);
		copyArray(getElement(_Index), getElement(_Index + _Length), this->length - _Index);
		copyArray(_Arr, getElement(_Index), _Length);
		this->length += _Length;
	}

	//Overwrites existing data with the data from the passed array.
	void overwrite(const size_t _Length, _T _Arr[], const size_t _Index)
	{
		reallocIfExceeds((this->length - _Length) + _Index);
		copyArray(_Arr, getElement(_Index), _Length);
		this->length += (this->length - _Length) + _Index;
	}

	//Adjusts the array's size to only fit the passed array and overwrites all existing data.
	void recreate(const size_t _Length, _T _Arr[])
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

	Pair<bool, size_t> find(const _T& _Obj)
	{
		for (size_t i = 0; i < this->length; i++)
		{
			if (_Obj == data[i])
			{
				return { true, i };
			}
		}
		
		return { false, 0 };
	}

	//Looks for object inside of the array and when it finds it, it deletes it.
	void eraseObject(_T& _Obj)
	{
		auto res = find(_Obj);
		if(res != ~0ULL)
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
	INLINE _T& operator[](const size_t index)
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
	INLINE const _T& operator[](const size_t index) const
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
	INLINE _T* getData() { return this->data; }

	//Returns a pointer to the allocated array.
	INLINE const _T* getData() const { return this->data; }
};
