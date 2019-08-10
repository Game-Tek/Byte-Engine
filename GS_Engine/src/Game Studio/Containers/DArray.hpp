#pragma once

#include "Core.h"

#include <cstdlib>
#include <cstring>

template <typename T, typename LT = size_t>
GS_CLASS DArray
{
	T* Data = nullptr;
	LT Capacity = 0;
	LT Length = 0;

private:
	T* allocate(const LT _elements)
	{
		return SCAST(T*, malloc(sizeof(T) * _elements));
	}

	T* copyLength(const LT _elements, void* _from)
	{
		void* dst;
		memcpy(dst, _from, sizeof(T) * _elements);
		return SCAST(T*, dst);
	}

public:
	DArray() = default;

	DArray(T _Data[], const LT _Length) : Data(allocate(_Length)), Capacity(_Length)
	{
		Data = copyLength(_Length, _Data);
	}

	~DArray()
	{
		free(Data);
	}

	T& operator[](const LT i)
	{
		return Data[i];
	}

	const T& operator[](const LT i) const
	{
		return Data[i];
	}

	T* data()
	{
		return  &Data;
	}

	[[nodiscard]] const T* data() const
	{
		return  &Data;
	}

	LT push_back(const T& _obj)
	{
		Data[Length] = _obj;

		return Length++;
	}

	LT push_back(const T* _obj)
	{
		Data[Length] = *_obj;

		return Length++;
	}

	LT length() const
	{
		return Length;
	}
};
