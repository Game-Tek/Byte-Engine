#pragma once

#include "DArray.hpp"
#include "Pair.h"

template <typename T>
class FHashMap
{
	DArray<T> array;

	INLINE size_t indexFromHash(const size_t hash) { return hash & (array.getCapacity() - 1); }

public:
	explicit FHashMap(const size_t Length) : array(Length)
	{
	}

	T* Insert(const T& val, const size_t hash)
	{
		return &(array[indexFromHash(hash)] = val);
	}

	void Remove(const size_t hash) { array[indexFromHash(hash)].~T(); }

	T& At(const size_t hash) { return array[indexFromHash(hash)]; }

	//Pair<bool, T*> Find(const size_t hash)
	//{
	//	if (array[indexFromHash(hash)])
	//	{
	//		return Pair<bool, T*>(true, &array[indexFromHash(hash)]);
	//	}
	//	else
	//	{
	//		return Pair<bool, T*>(false, nullptr);
	//	}
	//}
	//
	//Pair<bool, T*> TryPush(const T& val, const size_t hash)
	//{
	//	if(array[indexFromHash(hash)].First)
	//	{
	//		return Pair<bool, T*>(true, &array[indexFromHash(hash)].Second);
	//	}
	//	else
	//	{
	//		array[indexFromHash(hash)].Second = val;
	//		return Pair<bool, T*>(false, &array[indexFromHash(hash)].Second);
	//	}
	//}

	T* begin() { return array.begin(); }
	T* end() { return array.end(); }
};
