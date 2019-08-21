#pragma once

#include "Core.h"

#include <cstdlib>
#include <cstring>
#include <initializer_list>

template <typename T, typename LT = uint8>
class GS_EXPORT_ONLY DArray
{
	T* Data = nullptr;
	LT Capacity = 0;
	LT Length = 0;

private:
	static T* allocate(const LT _elements)
	{
		return SCAST(T*, malloc(sizeof(T) * _elements));
	}

	void copyLength(const LT _elements, void* _from)
	{
		auto i = memcpy(this->Data, _from, sizeof(T) * _elements);
	}

	void freeArray()
	{
		free(this->Data);
		this->Data = nullptr;
		return;
	}

public:
	DArray() = default;

	DArray(const std::initializer_list<T>& _List) : Capacity(_List.size()), Length(_List.size()), Data(allocate(_List.size()))
	{
		copyLength(this->Length, CCAST(T*, _List.begin()));
	}

	DArray(LT _Length) : Capacity(_Length), Length(_Length), Data(allocate(_Length))
	{
	}

	DArray(T _Data[], const LT _Length) : Data(allocate(_Length)), Capacity(_Length), Length(_Length)
	{
		copyLength(_Length, _Data);
	}

	DArray(const DArray<T>& _Other)
	{
		freeArray();
		this->Capacity = _Other.Capacity;
		this->Length = _Other.Length;
		this->Data = allocate(this->Capacity);
		copyLength(this->Capacity, _Other.Data);
	}

	~DArray()
	{
		freeArray();
	}

	DArray& operator=(const DArray<T>& _Other)
	{
		freeArray();
		this->Capacity = _Other.Capacity;
		this->Length = _Other.Length;
		this->Data = allocate(this->Capacity);
		copyLength(this->Capacity, _Other.Data);
		return *this;
	}

	T& operator[](const LT i)
	{
		return this->Data[i];
	}

	const T& operator[](const LT i) const
	{
		return this->Data[i];
	}

	T* data()
	{
		return this->Data;
	}

	[[nodiscard]] const T* data() const
	{
		return this->Data;
	}

	LT push_back(const T& _obj)
	{
		this->Data[this->Length] = _obj;

		return this->Length++;
	}

	LT push_back(const T* _obj)
	{
		this->Data[this->Length] = *_obj;

		return this->Length++;
	}

	[[nodiscard]] LT length() const
	{
		return this->Length;
	}
};
