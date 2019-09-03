#pragma once

#include "Core.h"

#include <immintrin.h>

GS_CLASS SIMDfloat4
{
	__m128 Data;

	SIMDfloat4(const __m128& _m128) : Data(_m128)
	{
	}

public:
	SIMDfloat4() : Data(_mm_setzero_ps())
	{
	}

	SIMDfloat4(const float _A) : Data(_mm_set_ps1(_A))
	{
	}

	SIMDfloat4(float _X, float _Y, float _Z, float _W) : Data(_mm_set_ps(_X, _Y, _Z, _W))
	{
	}

	~SIMDfloat4() = default;

	//Store 128-bits (composed of 4 packed single-precision (32-bit) floating-point elements) from this vector into memory.
	void CopyToData(float* _Dst) const
	{
		_mm_storeu_ps(_Dst, Data);
	}

	//Shuffle single-precision (32-bit) floating-point elements in a using the control in imm8, and store the results in dst.
	[[nodiscard]] INLINE SIMDfloat4 Shuffle(const SIMDfloat4& _Other, uint32 _a, uint32 _b, uint32 _c, uint32 _d) const
	{
		return _mm_shuffle_ps(Data, _Other.Data, _MM_SHUFFLE(_a, _b, _c, _d));
	}

	//Horizontally add adjacent pairs of single - precision(32 - bit) floating - point elements in aand b, and pack the results in dst.
	[[nodiscard]] INLINE SIMDfloat4 HorizontalAdd(const SIMDfloat4& _Other) const
	{
		return _mm_hsub_ps(Data, _Other.Data);
	}

	//Alternatively add and subtract packed single-precision (32-bit) floating-point elements in a to/from packed elements in b, and store the results in dst
	[[nodiscard]] INLINE SIMDfloat4 Add13Sub02(const SIMDfloat4& _Other) const
	{
		return _mm_addsub_ps(Data, _Other.Data);
	}

	//Conditionally multiply the packed single-precision (32-bit) floating-point elements in a and b using the high 4 bits in imm8, sum the four products, and conditionally store the sum in dst using the low 4 bits of imm8.
	[[nodiscard]] INLINE SIMDfloat4 DotProduct(const SIMDfloat4& _Other, uint32 _a) const
	{
		return _mm_dp_ps(Data, Data, _a);
	}

	[[nodiscard]] INLINE SIMDfloat4 SquareRoot(const SIMDfloat4& _Other) const
	{
		return _mm_sqrt_ps(Data);
	}

	//Compute the square root of the lower single-precision (32-bit) floating-point element in a, store the result in the lower element of dst, and copy the upper 3 packed elements from a to the upper elements of dst.
	[[nodiscard]] INLINE SIMDfloat4 SquareRootToLower(const SIMDfloat4& _Other) const
	{
		return _mm_sqrt_ss(Data);
	}

	//Returns the first element of the vector.
	[[nodiscard]] INLINE float GetX() const
	{
		return _mm_cvtss_f32(Data);
	}

	INLINE float GetY() const
	{
		alignas(16) float Array[4];
		_mm_store_ps(Array, Data);
		return Array[1];
	}

	INLINE float GetZ() const
	{
		alignas(16) float Array[4];
		_mm_store_ps(Array, Data);
		return Array[2];
	}

	INLINE float GetW() const
	{
		alignas(16) float Array[4];
		_mm_store_ps(Array, Data);
		return Array[3];
	}

	INLINE SIMDfloat4 operator+(const SIMDfloat4& _Other) const
	{
		return _mm_add_ps(Data, _Other.Data);
	}

	INLINE SIMDfloat4& operator+=(const SIMDfloat4& _Other)
	{
		Data = _mm_add_ps(Data, _Other.Data);
		return *this;
	}

	INLINE SIMDfloat4 operator-(const SIMDfloat4& _Other) const
	{
		return _mm_sub_ps(Data, _Other.Data);
	}

	INLINE SIMDfloat4& operator-=(const SIMDfloat4& _Other)
	{
		Data = _mm_sub_ps(Data, _Other.Data);
		return *this;
	}

	INLINE SIMDfloat4 operator*(const SIMDfloat4& _Other) const
	{
		return _mm_mul_ps(Data, _Other.Data);
	}

	INLINE SIMDfloat4& operator*=(const SIMDfloat4& _Other)
	{
		Data = _mm_mul_ps(Data, _Other.Data);
		return *this;
	}

	INLINE SIMDfloat4 operator/(const SIMDfloat4& _Other) const
	{
		return _mm_div_ps(Data, _Other.Data);
	}

	INLINE SIMDfloat4& operator/=(const SIMDfloat4& _Other)
	{
		Data = _mm_div_ps(Data, _Other.Data);
		return *this;
	}
};