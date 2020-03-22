#pragma once

#include "Core.h"

#include <cstdlib>
#include <cstring>
#include <initializer_list>

template <typename T>
class DArray final
{
	uint32 capacity = 0;
	uint32 length = 0;
	T* data = nullptr;

	static constexpr T* allocate(const uint32 _elements) { return static_cast<T*>(malloc(sizeof(T) * _elements)); }

	void copyLength(const uint32 _elements, void* _from)
	{
		memcpy(this->data, _from, sizeof(T) * _elements);
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
	typedef T* iterator;
	typedef const T* const_iterator;

	[[nodiscard]] iterator begin() { return this->data; }

	[[nodiscard]] iterator end() { return &this->data[this->length]; }

	[[nodiscard]] const_iterator begin() const { return this->data; }

	[[nodiscard]] const_iterator end() const { return &this->data[this->length]; }

	T& front() { return this->data[0]; }

	T& back() { return this->data[this->length]; }

	[[nodiscard]] const T& front() const { return this->data[0]; }

	[[nodiscard]] const T& back() const { return this->data[this->length]; }

	DArray() = default;

	constexpr DArray(const std::initializer_list<T>& _List) : capacity(_List.size()), length(_List.size()), data(allocate(_List.size()))
	{
		copyLength(this->length, const_cast<T*>(_List.begin()));
	}

	explicit DArray(const uint32 _Length) : capacity(_Length), length(0), data(allocate(_Length))
	{
	}

	DArray(T _Data[], const uint32 _Length) : data(allocate(_Length)), capacity(_Length), length(_Length)
	{
		copyLength(_Length, _Data);
	}

	DArray(const_iterator _Start, const_iterator _End) : capacity(_End - _Start), length(this->capacity),
	                                                     data(allocate(this->capacity))
	{
		copyToData(_Start, (_End - _Start) * sizeof(T));
	}

	DArray(const DArray<T>& _Other) : capacity(_Other.capacity), length(_Other.length),
	                                   data(allocate(this->capacity))
	{
		copyLength(this->capacity, _Other.data);
	}

	~DArray()
	{
		freeArray();
	}

	DArray& operator=(const DArray<T>& _Other)
	{
		freeArray();
		this->capacity = _Other.capacity;
		this->length = _Other.length;
		this->data = allocate(this->capacity);
		copyLength(this->capacity, _Other.data);
		return *this;
	}

	T& operator[](const uint32 i)
	{
		BE_ASSERT(i > this->capacity, "Out of Bounds! Requested index is greater than the array's allocated(current) size!")
		return this->data[i];
	}

	const T& operator[](const uint32 i) const
	{
		BE_ASSERT(i > this->capacity, "Out of Bounds! Requested index is greater than the array's allocated(current) size!")
		return this->data[i];
	}

	T* getData()
	{
		return this->data;
	}

	[[nodiscard]] const T* getData() const
	{
		return this->data;
	}

	uint32 push_back(const T& _obj)
	{
		this->data[this->length] = _obj;

		return this->length++;
	}

	uint32 push_back(const T* _obj)
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
	[[nodiscard]] size_t getSize() const { return this->capacity * sizeof(T); }
	//Returns the size in bytes the current length of the array takes up.
	[[nodiscard]] size_t getLengthSize() const { return this->length * sizeof(T); }
};
