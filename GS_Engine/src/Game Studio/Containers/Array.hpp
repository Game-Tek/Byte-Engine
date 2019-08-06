#pragma once

#include "Core.h"

template <typename T, size_t Size, typename LT = size_t>
GS_CLASS Array
{
	T Data[Size];
	LT Length = 0;

public:
	Array() = default;

	Array(T _Data[], const LT _Length) : Data(_Data), Length(_Length)
	{
	}

	T& operator[](const LT i)
	{
		return Data[i];
	}

	const T& operator[](const LT i) const
	{
		return Data[i];
	}

	void setLength(const LT _length) { Length = _length; }

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

	[[nodiscard]] LT capacity() const
	{
		return Size;
	}
};