#pragma once

#include "Core.h"

#include <immintrin.h>

GS_CLASS int4
{
	__m128i Data;

public:
	INLINE int4 operator+(const int4& _Other) const
	{
		return _mm_add_ps(Data, _Other.Data);
	}
	
	INLINE int4& operator+=(const int4& _Other)
	{
		Data = _mm_add_ps(Data, _Other.Data);
		return *this;
	}
	
	INLINE int4 operator-(const int4& _Other) const
	{
		return _mm_sub_ps(Data, _Other.Data);
	}
	
	INLINE int4& operator-=(const int4& _Other)
	{
		Data = _mm_sub_ps(Data, _Other.Data);
		return *this;
	}
	
	INLINE int4 operator*(const int4& _Other) const
	{
		return _mm_mul_ps(Data, _Other.Data);
	}
	
	INLINE int4& operator*=(const int4& _Other)
	{
		Data = _mm_mul_ps(Data, _Other.Data);
		return *this;
	}
	
	INLINE int4 operator/(const int4& _Other) const
	{
		return _mm_div_ps(Data, _Other.Data);
	}
	
	INLINE int4& operator/=(const int4& _Other)
	{
		Data = _mm_div_ps(Data, _Other.Data);
		return *this;
	}
};