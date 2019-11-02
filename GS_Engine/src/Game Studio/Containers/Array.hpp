#pragma once

#include "Core.h"

#include <initializer_list>

template <typename T, size_t Size, typename LT = uint8>
class Array
{
	T Data[Size];
	LT Length = 0;

	void CopyToData(const void* _Src, const LT _Length)
	{
		memcpy(this->Data, _Src, _Length * sizeof(T));
	}

public:
	Array() = default;

	Array(const std::initializer_list<T> _InitList) : Length(_InitList.size())
	{
		CopyToData(_InitList.begin(), this->Length);
	}

	explicit Array(const LT _Length) : Length(_Length)
	{
	}

	Array(const LT _Length, T _Data[]) : Data(), Length(_Length)
	{
		CopyToData(_Data, Length);
	}

	T* begin() { return this->Data; }
	T* end() { return &this->Data[Size] + 1; }
	
	T& operator[](const LT i)
	{
		return this->Data[i];
	}

	const T& operator[](const LT i) const
	{
		return this->Data[i];
	}

	void setLength(const LT _length) { Length = _length; }

	const T* getData()
	{
		return this->Data;
	}

	[[nodiscard]] const T* getData() const
	{
		return this->Data;
	}

	LT push_back(const T& _obj)
	{
		CopyToData(&_obj, 1);

		return ++this->Length;
	}

	//LT push_back(const T* _obj)
	//{
	//	this->Data[this->Length] = *_obj;
	//
	//	return this->Length++;
	//}

	[[nodiscard]] LT getLength() const	{ return this->Length; }

	[[nodiscard]] LT getCapacity() const { return Size; }
};
