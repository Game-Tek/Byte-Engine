#pragma once

#include "Core.h"

template<typename T, uint32 L, typename S = uint32>
GS_CLASS Array
{
public:
	Array()
	{
	}

	S GetLength() const
	{
		return this->Length;
	}

	T * GetData()
	{
		return this->Data;
	}

	const T * GetData() const
	{
		return this->Data;
	}

	void PushBack(const T & Obj)
	{
		this->Data[Length] = Obj;

		this->Length += 1;
	}

	void PopBack()
	{
		this->Length -= 1;
	}

private:
	S Length;
	T Data[L];
};