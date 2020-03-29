#pragma once

#include "Core.h"
#include <cstdlib>
#include <cstring>
#include <type_traits>
#include <initializer_list>

template <typename T>
class FVector
{
public:
	typedef uint32 length_type;

	typedef T* iterator;
	typedef const T* const_iterator;
	typedef T value_type;
private:
	length_type capacity = 0;
	length_type length = 0;

	T* data = nullptr;

	/**
	 * \brief Copies data from from to to.
	 * \param from Pointer from where to grab the data.
	 * \param to Pointer to write the data to.
	 * \param elementCount How many elements of this vector's T to write to to.
	 */
	static void copyArray(const T* from, T* to, const length_type elementCount)
	{
		memcpy(to, from, elementCount * sizeof(T));
	}

	/**
	 * \brief Allocates a new a array of type T with enough space to hold elementCount elements.
	 * \param elementCount How many elements of this vector's T to allocate space for.
	 * \return T pointer to the newly allocated memory.
	 */
	static T* allocate(const length_type elementCount) { return static_cast<T*>(malloc(elementCount * sizeof(T))); }

	/**
	 * \brief Deletes the data found at this vector's data and sets data as nullptr.
	 */
	void freeData()
	{
		GTSL_ASSERT(this->data == nullptr, "Data is nullptr.")
		free(this->data);
		this->data = nullptr;
	}

	/**
	 * \brief Reallocates this->data to a new array if (this->length + additionalElements) exceeds the allocated space.\n
	 * Also deletes the data found at the old data.
	 * Growth of the array is geometric.
	 * \param additionalElements How many elements of this vector's T are you trying to check if fit in the already allocated array.\n
	 * Number can be negative.
	 */
	void reallocIfExceeds(const int64 additionalElements)
	{
		if (this->length + additionalElements > this->capacity)
		{
			const length_type new_capacity = this->length * 1.5;
			T* new_data = allocate(new_capacity);
			copyArray(this->data, new_data, this->capacity);
			freeData();
			this->capacity = new_capacity;
			this->data = new_data;
		}
	}
	
	/**
	 * \brief Returns an iterator to an specified index. DOES NOT CHECK FOR BOUNDS, but underlying getter does, only in debug builds.
	 * \param index Index to the element to be retrieved.
	 * \return iterator to the element at index.
	 */
	iterator getIterator(const length_type index) { return &this->data[index]; }
public:
	//Constructs a new FVector.
	FVector() : capacity(0), length(0), data(nullptr)
	{
	}

	//Constructs a new FVector allocating space for the quantity of elements specified in length.
	explicit FVector(const length_type capacity) : capacity(capacity), length(0), data(allocate(this->capacity))
	{
	}

	explicit FVector(const length_type capacity, const length_type length) : capacity(capacity), length(length),
	                                                                data(allocate(this->capacity))
	{
	}

	FVector(const length_type length, const T* array) : capacity(length), length(length), data(allocate(this->capacity))
	{
		copyArray(array, this->data, length);
	}
	
	constexpr FVector(const std::initializer_list<T>& initializerList) : capacity(initializerList.end() - initializerList.begin()),
		length(this->capacity),
		data(allocate(this->capacity))
	{
		copyArray(initializerList.begin(), this->data, this->length);
	}

	FVector(const_iterator start, const_iterator end) : capacity(end - start), length(end - start), data(allocate(this->capacity))
	{
		copyArray(start, this->data);
	}

	//Constructs a new FVector from another FVector.
	FVector(const FVector& other) : capacity(other.capacity), length(other.length), data(allocate(this->capacity))
	{
		copyArray(other.data, this->data, this->length);
	}

	//Assigns this object the data of the passed in FVector.
	FVector& operator=(const FVector& other)
	{
		GTSL_ASSERT(this == &other, "Assigning to self is not allowed!")
		reallocIfExceeds(other.length - this->length);
		copyArray(other.data, this->data, other.length);
		this->length = other.length;
		return *this;
	}

	~FVector()
	{
		for(auto& e : *this) { e.~T(); }
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

	void resize(const length_type count)
	{
		reallocIfExceeds(count - this->length);
		this->length = count;
		return;
	}

	void init(const length_type count)
	{
		this->data = allocate(count);
		this->capacity = count;
		this->length = 0;
		return;
	}

	void init(const length_type count, const T* data)
	{
		this->data = allocate(count);
		copyArray(data, this->data, count);
		this->capacity = count;
		this->length = count;
		return;
	}

	void shrink(const length_type count)
	{
		this->capacity = count;
		this->length = count;
		T* buffer = allocate(this->capacity);
		copyArray(this->data, buffer, this->length);
		freeData();
		this->data = buffer;
		return;
	}

	//Places the passed in element at the end of the array.
	length_type push_back(const T& obj)
	{
		reallocIfExceeds(1);
		//copyArray(&obj, getIterator(this->length), 1);
		::new(this->data + this->length) T(obj);
		return this->length += 1;
	}

	//Places the passed in array at the end of the array.
	length_type push_back(const length_type length, const T arr[])
	{
		reallocIfExceeds(length);
		copyArray(arr, getIterator(this->length), length);
		return this->length += length;
	}

	//Places the passed in FVector at the end of the array.
	length_type push_back(const FVector& other)
	{
		reallocIfExceeds(this->length - other.length);
		copyArray(other.data, getIterator(this->length), other.length);
		return this->length += other.length;
	}

	template <typename... Args>
	length_type emplace_back(Args&&... args)
	{
		reallocIfExceeds(1);
		::new(this->data + this->length) T(std::forward<Args>(args) ...);
		return this->length += 1;
	}

	//Deletes the array's last element.
	void pop_back()
	{
		if (this->length != 0)
		{
			this->data[this->length].~T();
			this->length -= 1;
		}
	}

	//Makes space at the specified index.
	void make_space(length_type index, length_type length)
	{
		reallocIfExceeds(length);
		copyArray(getIterator(index), getIterator(index + length), this->length - index);
		this->length += length;
	}
	
	//Places the passed in element at the specified index and shifts the rest of the array forward to fit it in.
	length_type push(length_type index, const T& obj)
	{
		reallocIfExceeds(1);
		copyArray(getIterator(index), getIterator(index + 1), this->length - index);
		::new(this->data + this->length) T(obj);
		return this->length += 1;
	}

	//Places the passed array at the specified index and shifts the rest of the array forward to fit it in.
	void push(const length_type length, T arr[], const length_type index)
	{
		reallocIfExceeds(length);
		copyArray(getIterator(index), getIterator(index + length), this->length - index);
		copyArray(arr, getIterator(index), length);
		this->length += length;
	}

	//Overwrites existing data with the data from the passed array.
	void overwrite(const length_type length, const T arr[], const length_type index)
	{
		reallocIfExceeds((this->length - length) + index);
		copyArray(arr, getIterator(index), length);
		this->length += (this->length - length) + index;
	}

	//Adjusts the array's size to only fit the passed array and overwrites all existing data.
	void recreate(const length_type length, const T arr[])
	{
		reallocIfExceeds(length - this->length);
		copyArray(arr, this->data, length);
		this->length = length;
		return;
	}

	void place(const length_type index, const T& obj) { ::new(this->data + index) T(obj); }
	template <typename... Args>
	void emplace(const length_type index, Args&&... args) { ::new(this->data + index) T(std::forward<Args>(args) ...); }
	void destroy(const length_type index) { this->data[index].~T(); }
	
	//Deletes the element at the specified index and shifts the array backwards to fill the empty space.
	void pop(const length_type index)
	{
		this->data[index].~T();
		copyArray(getIterator(index + 1), getIterator(index), this->length - index);
		this->length -= 1;
	}

	//Deletes all elements between index and index + length and shifts the entire array backwards to fill the empty space.
	void popRange(const length_type index, const length_type length)
	{
		copyArray(getIterator(index + length), getIterator(index), this->length - index);
		this->length -= length;
	}

	iterator find(const T& obj)
	{
		for (length_type i = 0; i < this->length; i++)
		{
			if (obj == data[i])
			{
				return getIterator(i);
			}
		}

		return this->end();
	}

	//Looks for object inside of the array and when it finds it, it deletes it.
	void eraseObject(T& obj)
	{
		auto res = find(obj);
		if (res != this->end())
		{
			pop(res);
		}
	}

	//Returns the element at the specified index. ONLY CHECKS FOR OUT OF BOUNDS IN DEBUG BUILDS.
	T& operator[](const length_type index)
	{
		GTSL_ASSERT(index < this->length, "Entered index is not accessible, array is not as large.")
		return this->data[index];
	}

	//Returns the element at the specified index. ONLY CHECKS FOR OUT OF BOUNDS IN DEBUG BUILDS.
	const T& operator[](const length_type index) const
	{
		GTSL_ASSERT(index < this->length, "Entered index is not accessible, array is not as large.")
		return this->data[index];
	}

	T& at(const length_type index) { return this->data[index]; }

	//Returns the occupied elements count.
	[[nodiscard]] length_type getLength() const { return this->length; }

	//Returns the total allocated elements count. 
	[[nodiscard]] length_type getCapacity() const { return this->capacity; }

	//Returns a pointer to the allocated array.
	T* getData() { return this->data; }

	//Returns a pointer to the allocated array.
	const T* getData() const { return this->data; }

	[[nodiscard]] size_t getLengthSize() const { return this->length * sizeof(T); }
};
