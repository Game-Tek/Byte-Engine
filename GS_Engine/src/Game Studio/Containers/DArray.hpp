#pragma once

#include "Core.h"

#include <cstdlib>
#include <cstring>
#include <initializer_list>

template <typename _T>
class DArray
{
	uint32 capacity = 0;
	uint32 length = 0;
	_T* data = nullptr;

	static _T* allocate(const uint32 _elements)
	{
		//auto align = alignof(T);
		return SCAST(_T*, malloc(sizeof(_T) * _elements));
		//return SCAST(T*, _aligned_malloc(_elements * sizeof(T), alignof(T)));
	}

	void copyLength(const uint32 _elements, void* _from)
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
	typedef _T* iterator;
	typedef const _T* const_iterator;

	[[nodiscard]] iterator begin() { return this->data; }

	[[nodiscard]] iterator end() { return &this->data[this->length]; }

	[[nodiscard]] const_iterator begin() const { return this->data; }

	[[nodiscard]] const_iterator end() const { return &this->data[this->length]; }

	_T& front() { return this->data[0]; }

	_T& back() { return this->data[this->length]; }

	[[nodiscard]] const _T& front() const { return this->data[0]; }

	[[nodiscard]] const _T& back() const { return this->data[this->length]; }

	DArray() = default;

	DArray(const std::initializer_list<_T>& _List) : capacity(_List.size()), length(_List.size()),
	                                                 data(allocate(_List.size()))
	{
		copyLength(this->length, CCAST(_T*, _List.begin()));
	}

	explicit DArray(const uint32 _Length) : capacity(_Length), length(0), data(allocate(_Length))
	{
	}

	DArray(_T _Data[], const uint32 _Length) : data(allocate(_Length)), capacity(_Length), length(_Length)
	{
		copyLength(_Length, _Data);
	}

	DArray(const_iterator _Start, const_iterator _End) : capacity(_End - _Start), length(this->capacity),
	                                                     data(allocate(this->capacity))
	{
		copyToData(_Start, (_End - _Start) * sizeof(_T));
	}

	DArray(const DArray<_T>& _Other) : capacity(_Other.capacity), length(_Other.length),
	                                   data(allocate(this->capacity))
	{
		copyLength(this->capacity, _Other.data);
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

	_T& operator[](const uint32 i)
	{
		GS_DEBUG_ONLY(GS_ASSERT(i > this->capacity))
		return this->data[i];
	}

	const _T& operator[](const uint32 i) const
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

	uint32 push_back(const _T& _obj)
	{
		this->data[this->length] = _obj;

		return this->length++;
	}

	uint32 push_back(const _T* _obj)
	{
		this->data[this->length] = *_obj;

		return this->length++;
	}

	[[nodiscard]] uint32 getLength() const
	{
		return this->length;
	}

	[[nodiscard]] uint32 getCapacity() const
	{
		return this->capacity;
	}

	void resize(uint32 _NewLength)
	{
		this->length = _NewLength;
	}

	//Returns the size in bytes the currently allocated array takes up.
	[[nodiscard]] size_t getSize() const { return this->capacity * sizeof(_T); }
	//Returns the size in bytes the current length of the array takes up.
	[[nodiscard]] size_t getLengthSize() const { return this->length * sizeof(_T); }
};
