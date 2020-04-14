#pragma once

#include "Core.h"

#include <initializer_list>
#include "Memory.h"
#include "Assert.h"

namespace GTSL
{
	template <typename T, size_t CAPACITY, typename LT = uint32>
	class Array
	{
		byte data[CAPACITY * sizeof(T)];
		LT length = 0;

		constexpr void copyToData(const void* from, const LT length) noexcept
		{
			Memory::CopyMemory(length * sizeof(T), from, this->data);
		}

	public:
		typedef T* iterator;
		typedef const T* const_iterator;

		[[nodiscard]] constexpr iterator begin() noexcept { return reinterpret_cast<iterator>(this->data); }

		[[nodiscard]] constexpr iterator end() noexcept { return reinterpret_cast<iterator>(&this->data[this->length]); }

		[[nodiscard]] constexpr const_iterator begin() const noexcept { return reinterpret_cast<const_iterator>(this->data); }

		[[nodiscard]] constexpr const_iterator end() const noexcept { return reinterpret_cast<const_iterator>(&this->data[this->length]); }

		constexpr T& front() noexcept { return this->data[0]; }

		constexpr T& back() noexcept { return this->data[this->length]; }

		[[nodiscard]] constexpr const T& front() const noexcept { return this->data[0]; }

		[[nodiscard]] constexpr const T& back() const noexcept { return this->data[this->length]; }

		constexpr Array() noexcept = default;

		constexpr Array(const std::initializer_list<T>& list) noexcept : length(list.size())
		{
			copyToData(list.begin(), this->length);
		}

		constexpr explicit Array(const LT length) noexcept : length(length)
		{
		}

		constexpr Array(const LT length, T array[]) noexcept : length(length)
		{
			copyToData(array, length);
		}

		constexpr Array(const Array& other) noexcept : length(length)
		{
			copyToData(other.data, other.length);
		}

		constexpr Array(Array&& other) noexcept : length(length)
		{
			copyToData(other.data, other.length);
			for (auto& e : other) { e.~T(); }
			other.length = 0;
		}

		constexpr Array& operator=(const Array& other) noexcept
		{
			copyToData(other.data, other.length);
			length = other.length;
			return *this;
		}

		constexpr Array& operator=(Array&& other) noexcept
		{
			copyToData(other.data, other.length);
			length = other.length;
			for (auto& e : other) { e.~T(); }
			other.length = 0;
			return *this;
		}

		~Array()
		{
			for (auto& e : *this) { e.~T(); }
		}

		constexpr T& operator[](const LT i) noexcept
		{
			GTSL_ASSERT(i > CAPACITY, "Out of Bounds! Requested index is greater than the Array's statically allocated size!");
			return reinterpret_cast<T&>(this->data[i]);
		}

		constexpr const T& operator[](const LT i) const noexcept
		{
			GTSL_ASSERT(i > CAPACITY, "Out of Bounds! Requested index is greater than the Array's statically allocated size!");
			return reinterpret_cast<T&>(const_cast<byte&>(this->data[i]));
		}

		constexpr T* GetData() noexcept { return reinterpret_cast<T*>(&this->data); }

		[[nodiscard]] constexpr const T* GetData() const noexcept { return reinterpret_cast<const T*>(this->data); }

		constexpr LT PushBack(const T& obj) noexcept
		{
			GTSL_ASSERT((this->length + 1) > CAPACITY, "Array is not long enough to insert any more elements!");
			::new(this->data + this->length) T(obj);
			return ++this->length;
		}

		template<typename... ARGS>
		constexpr LT EmplaceBack(ARGS&&... args)
		{
			GTSL_ASSERT((this->length + 1) > CAPACITY, "Array is not long enough to insert any more elements!");
			::new(this->data + this->length) T(GTSL::MakeForwardReference<ARGS>(args) ...);
			return ++this->length;
		}

		constexpr void Resize(LT size)
		{
			GTSL_ASSERT(size > CAPACITY, "Requested size for array Resize is greater than Array's statically allocated size!");
			this->length = size;
		}

		constexpr void PopBack()
		{
			GTSL_ASSERT(this->length == 0, "Array's length is already 0. Cannot pop any more elements!");
			reinterpret_cast<T&>(this->data[this->length]).~T();
			--this->length;
		}

		[[nodiscard]] constexpr LT GetLength() const noexcept { return this->length; }

		[[nodiscard]] constexpr LT GetCapacity() const noexcept { return CAPACITY; }
	};
}