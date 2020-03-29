#pragma once

#include "Core.h"

template<typename T>
class FlatHashMap
{
	using key_type = uint64;
	
	uint32 size = 0;
	
	T* data = nullptr;
	uint16* deltas = nullptr;

	static constexpr uint32 modulo(const key_type key, const uint32 size) { return key & (size - 1); }

	void tryResize()
	{
		
	}
public:
	void Insert(const key_type key, const T& obj)
	{
		const auto index = modulo(key, size);

		data[index](obj);
	}
};