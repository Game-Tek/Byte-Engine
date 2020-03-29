#pragma once

#include "Core.h"

#include <initializer_list>
#include <forward_list>

template <typename T, size_t CAPACITY, typename LT = uint32>
class Array
{
	byte data[CAPACITY * sizeof(T)];
	LT length = 0;

	void copyToData(const void* _Src, const LT _Length)
	{
		memcpy(this->data, _Src, _Length * sizeof(T));
	}

public:
	typedef T* iterator;
	typedef const T* const_iterator;

	[[nodiscard]] iterator begin() { return reinterpret_cast<iterator>(this->data); }

	[[nodiscard]] iterator end() { return reinterpret_cast<iterator>(&this->data[this->length]); }

	[[nodiscard]] const_iterator begin() const { return reinterpret_cast<const_iterator>(this->data); }

	[[nodiscard]] const_iterator end() const { return reinterpret_cast<const_iterator>(&this->data[this->length]); }

	T& front() { return this->data[0]; }

	T& back() { return this->data[this->length]; }

	[[nodiscard]] const T& front() const { return this->data[0]; }

	[[nodiscard]] const T& back() const { return this->data[this->length]; }

	Array() = default;

	constexpr Array(const std::initializer_list<T>& list) : length(list.size())
	{
		copyToData(list.begin(), this->length);
	}

	explicit Array(const LT length) : length(length)
	{
	}

	Array(const LT length, T array[]) : data(), length(length)
	{
		copyToData(array, length);
	}

	~Array()
	{
		for (auto& e : *this) { e.~T(); }
	}

	T& operator[](const LT i)
	{
		GTSL_ASSERT(i > CAPACITY, "Out of Bounds! Requested index is greater than the Array's statically allocated size!")
		return reinterpret_cast<T&>(this->data[i]);
	}

	const T& operator[](const LT i) const
	{
		GTSL_ASSERT(i > CAPACITY, "Out of Bounds! Requested index is greater than the Array's statically allocated size!")
		return reinterpret_cast<T&>(const_cast<byte&>(this->data[i]));
	}

	T* getData() { return reinterpret_cast<T*>(&this->data); }

	[[nodiscard]] const T* getData() const { return reinterpret_cast<const T*>(this->data); }

	LT push_back(const T& obj)
	{
		GTSL_ASSERT((this->length + 1) > CAPACITY, "Array is not long enough to insert any more elements!")
		::new(this->data + this->length) T(obj);
		return ++this->length;
	}

	template<typename... ARGS>
	LT emplace_back(ARGS&&... args)
	{
		GTSL_ASSERT((this->length + 1) > CAPACITY, "Array is not long enough to insert any more elements!")
		::new(this->data + this->length) T(std::forward<ARGS>(args) ...);
		return ++this->length;
	}

	void resize(size_t size)
	{
		GTSL_ASSERT(size > CAPACITY, "Requested size for array resize is greater than Array's statically allocated size!")
		this->length = size;
	}
	
	void pop_back()
	{
		GTSL_ASSERT(this->length == 0, "Array's length is already 0. Cannot pop any more elements!")
		reinterpret_cast<T&>(this->data[this->length]).~T();
		--this->length;
	}

	[[nodiscard]] LT getLength() const { return this->length; }

	[[nodiscard]] LT getCapacity() const { return CAPACITY; }
};
