#pragma once

#include "Core.h"

#include <initializer_list>
#include <forward_list>

template <typename T, size_t Size, typename LT = uint32>
class Array
{
	byte data[Size * sizeof(T)];
	LT length = 0;

	void CopyToData(const void* _Src, const LT _Length)
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

	void resize(size_t size)
	{
		BE_ASSERT(size > Size, "Requested size for array resize is greater than Array's statically allocated size!")
		this->length = size;
	}

	Array() = default;
	
	~Array()
	{
		for (LT i = 0; i < this->length; ++i)
		{
			reinterpret_cast<T&>(this->data[i]).~T();
		}
	}

	constexpr Array(const std::initializer_list<T>& _InitList) : length(_InitList.size())
	{
		CopyToData(_InitList.begin(), this->length);
	}

	explicit Array(const LT _Length) : length(_Length)
	{
	}

	Array(const LT _Length, T _Data[]) : data(), length(_Length)
	{
		CopyToData(_Data, length);
	}

	T& operator[](const LT i)
	{
		BE_ASSERT(i > Size, "Out of Bounds! Requested index is greater than the Array's statically allocated size!")
		return reinterpret_cast<T&>(this->data[i]);
	}

	const T& operator[](const LT i) const
	{
		BE_ASSERT(i > Size, "Out of Bounds! Requested index is greater than the Array's statically allocated size!")
		return reinterpret_cast<T&>(const_cast<byte&>(this->data[i]));
	}

	T* getData() { return reinterpret_cast<T*>(&this->data); }

	[[nodiscard]] const T* getData() const { return reinterpret_cast<const T*>(this->data); }

	LT push_back(const T& obj)
	{
		BE_ASSERT((this->length + 1) > Size, "Array is not long enough to insert any more elements!")
		::new(this->data + this->length) T(obj);
		return ++this->length;
	}

	template<typename... Args>
	LT emplace_back(Args&&... args)
	{
		BE_ASSERT((this->length + 1) > Size, "Array is not long enough to insert any more elements!")
		::new(this->data + this->length) T(std::forward<Args>(args) ...);
		return ++this->length;
	}

	void pop_back()
	{
		BE_ASSERT(this->length == 0, "Array's length is already 0. Cannot pop any more elements!")
		reinterpret_cast<T&>(this->data[this->length]).~T();
		--this->length;
	}

	[[nodiscard]] LT getLength() const { return this->length; }

	[[nodiscard]] LT getCapacity() const { return Size; }
};
