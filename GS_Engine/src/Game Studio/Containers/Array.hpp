#pragma once

#include "Core.h"

#include <initializer_list>

template <typename T, size_t Size, typename LT = uint8>
class GS_EXPORT_ONLY Array
{
	char Data[Size * sizeof(T)];
	LT Length = 0;

	void CopyToData(const void* _Src)
	{
		memcpy(this->Data, _Src, this->Length * sizeof(T));
	}

	void CopyToData(const void* _Src, const LT _Length)
	{
		memcpy(this->Data, _Src, _Length * sizeof(T));
	}

public:
	Array() = default;

	Array(const std::initializer_list<T> _InitList) : Length(_InitList.size())
	{
		CopyToData(_InitList.begin());
	}

	explicit Array(const LT _Length) : Length(_Length)
	{
	}

	Array(T _Data[], const LT _Length) : Data(_Data), Length(_Length)
	{
	}

	T& operator[](const LT i)
	{
		return RCAST(T&, this->Data[i]);
	}

	const T& operator[](const LT i) const
	{
		return RCAST(T&, *CCAST(char*, &this->Data[i]));
	}

	void setLength(const LT _length) { Length = _length; }

	const T* data()
	{
		return RCAST(T*, this->Data);
	}

	[[nodiscard]] const T* data() const
	{
		return RCAST(T*, this->Data);
	}

	LT push_back(const T& _obj)
	{
		CopyToData(&_obj, 1);

		return this->Length++;
	}

	//LT push_back(const T* _obj)
	//{
	//	this->Data[this->Length] = *_obj;
	//
	//	return this->Length++;
	//}

	[[nodiscard]] LT length() const
	{
		return this->Length;
	}

	[[nodiscard]] LT capacity() const
	{
		return Size;
	}
};
