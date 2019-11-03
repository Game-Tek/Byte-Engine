#pragma once

#include "Core.h"

#include <initializer_list>

template <typename _T, size_t _Size, typename _LT = uint8>
class Array
{
	_T data[_Size];
	_LT length = 0;

	void CopyToData(const void* _Src, const _LT _Length)
	{
		memcpy(this->data, _Src, _Length * sizeof(_T));
	}

public:
	typedef _T* iterator;
	typedef const _T* const_iterator;

	[[nodiscard]] iterator begin() { return this->data; }

	[[nodiscard]] iterator end() { return &this->data[this->length]; }

	[[nodiscard]] const_iterator begin() const { return this->data; }

	[[nodiscard]] const_iterator end() const { return &this->data[this->length]; }

	_T& front() { return this->data[0]; }

	_T& back() { return this->data[this->length]; }

	[[nodiscard]] const _T& front() const { return this->data[0]; }

	[[nodiscard]] const _T& back() const { return this->data[this->length]; }
	
	Array() = default;

	Array(const std::initializer_list<_T>& _InitList) : length(_InitList.size())
	{
		CopyToData(_InitList.begin(), this->length);
	}

	explicit Array(const _LT _Length) : length(_Length)
	{
	}

	Array(const _LT _Length, _T _Data[]) : data(), length(_Length)
	{
		CopyToData(_Data, length);
	}
	
	_T& operator[](const _LT i)
	{
		GS_DEBUG_ONLY(GS_ASSERT(i > _Size))
		return this->data[i];
	}

	const _T& operator[](const _LT i) const
	{
		GS_DEBUG_ONLY(GS_ASSERT(i > _Size))
		return this->data[i];
	}

	void setLength(const _LT _length) { length = _length; }

	const _T* getData()
	{
		return this->data;
	}

	[[nodiscard]] const _T* getData() const
	{
		return this->data;
	}

	_LT push_back(const _T& _obj)
	{
		CopyToData(&_obj, 1);

		return ++this->length;
	}

	//LT push_back(const T* _obj)
	//{
	//	this->Data[this->Length] = *_obj;
	//
	//	return this->Length++;
	//}

	[[nodiscard]] _LT getLength() const	{ return this->length; }

	[[nodiscard]] _LT getCapacity() const { return _Size; }
};
