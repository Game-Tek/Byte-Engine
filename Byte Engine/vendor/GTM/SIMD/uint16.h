#pragma once

#include "Core.h"

#include <immintrin.h>

GS_CLASS int4
{
	__m128i Data;

public:
	INLINE int4 operator+(const int4& other) const
	{
		return _mm_add_ps(Data, other.Data);
	}
	
	INLINE int4& operator+=(const int4& other)
	{
		Data = _mm_add_ps(Data, other.Data);
		return *this;
	}
	
	INLINE int4 operator-(const int4& other) const
	{
		return _mm_sub_ps(Data, other.Data);
	}
	
	INLINE int4& operator-=(const int4& other)
	{
		Data = _mm_sub_ps(Data, other.Data);
		return *this;
	}
	
	INLINE int4 operator*(const int4& other) const
	{
		return _mm_mul_ps(Data, other.Data);
	}
	
	INLINE int4& operator*=(const int4& other)
	{
		Data = _mm_mul_ps(Data, other.Data);
		return *this;
	}
	
	INLINE int4 operator/(const int4& other) const
	{
		return _mm_div_ps(Data, other.Data);
	}
	
	INLINE int4& operator/=(const int4& other)
	{
		Data = _mm_div_ps(Data, other.Data);
		return *this;
	}
};