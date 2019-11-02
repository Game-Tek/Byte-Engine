#pragma once

#include "Core.h"

#include <cstdlib>
#include <cstring>
#include <initializer_list>

template <typename _T, typename _LT = uint8>
class DArray
{
	typedef _T* iterator;
	typedef const _T* const_iterator;

	_LT capacity = 0;
	_LT length = 0;
	_T* data = nullptr;

	static _T* allocate(const _LT _elements)
	{
		//auto align = alignof(T);
		return SCAST(_T*, malloc(sizeof(_T) * _elements));
		//return SCAST(T*, _aligned_malloc(_elements * sizeof(T), alignof(T)));
	}

	void copyLength(const _LT _elements, void* _from)
	{
		memcpy(this->data, _from, sizeof(_T) * _elements);
	}

	void copyToData(const void* _from, size_t _size)
	{
		memcpy(this->data, _from, _size);
	}

	void freeArray()
	{
		free(this->data);
		//_aligned_free(this->Data);
		this->data = nullptr;
		return;
	}

public:
	DArray() = default;

	DArray(const std::initializer_list<_T>& _List) : capacity(_List.size()), length(_List.size()), data(allocate(_List.size()))
	{
		copyLength(this->length, CCAST(_T*, _List.begin()));
	}

	explicit DArray(const _LT _Length) : capacity(_Length), length(0), data(allocate(_Length))
	{
	}

	DArray(_T _Data[], const _LT _Length) : data(allocate(_Length)), capacity(_Length), length(_Length)
	{
		copyLength(_Length, _Data);
	}

	DArray(const_iterator _Start, const_iterator _End) : capacity(_End - _Start), length(this->capacity), data(allocate(this->capacity))
	{
		copyToData(_Start, (_End - _Start) * sizeof(_T));
	}

	DArray(const DArray<_T>& _Other) : capacity(_Other.getCapacity), length(_Other.getLength), data(allocate(this->capacity))
	{
		copyLength(this->capacity, _Other.getData);
	}

	~DArray()
	{
		freeArray();
	}

	DArray& operator=(const DArray<_T>& _Other)
	{
		freeArray();
		this->capacity = _Other.capacity;
		this->length = _Other.length;
		this->data = allocate(this->capacity);
		copyLength(this->capacity, _Other.data);
		return *this;
	}

	_T& operator[](const _LT i)
	{
		GS_DEBUG_ONLY(GS_ASSERT(i > this->capacity))
		return this->data[i];
	}

	const _T& operator[](const _LT i) const
	{
		GS_DEBUG_ONLY(GS_ASSERT(i > this->capacity))
		return this->data[i];
	}

	_T* getData()
	{
		return this->data;
	}

	[[nodiscard]] const _T* getData() const
	{
		return this->data;
	}

	_LT push_back(const _T& _obj)
	{
		this->data[this->length] = _obj;

		return this->length++;
	}

	_LT push_back(const _T* _obj)
	{
		this->data[this->length] = *_obj;

		return this->length++;
	}

	[[nodiscard]] _LT getLength() const
	{
		return this->length;
	}

	[[nodiscard]] _LT getCapacity() const
	{
		return this->capacity;
	}

	void resize(_LT _NewLength)
	{
		this->length = _NewLength;
	}

	//Returns the size in bytes the currently allocated array takes up.
	[[nodiscard]] size_t getSize() const { return this->capacity * sizeof(_T); }
	//Returns the size in bytes the current length of the array takes up.
	[[nodiscard]] size_t getLengthSize() const { return this->length * sizeof(_T); }
};
