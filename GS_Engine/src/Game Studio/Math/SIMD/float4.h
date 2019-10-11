#pragma once

#include "Core.h"

#include <immintrin.h>

class GS_API float4
{
	__m128 Data;

	float4(const __m128& _m128) : Data(_m128)
	{
	}

public:
	float4() : Data(_mm_setzero_ps())
	{
	}

	explicit float4(const float _A) : Data(_mm_set_ps1(_A))
	{
	}

	float4(float _X, float _Y, float _Z, float _W) : Data(_mm_set_ps(_X, _Y, _Z, _W))
	{
	}

	float4(const float* _Data) : Data(_mm_loadu_ps(_Data))
	{
	}

	~float4() = default;

	operator __m128() const { return Data; }

	void SetAligned(const float* _Data) { Data = _mm_load_ps(_Data); }
	void SetUnaligned(const float* _Data) { Data = _mm_loadu_ps(_Data); }

	//Assumes aligned data.
	float4& operator=(const float* _Data) { Data = _mm_load_ps(_Data); return *this; }

	//Store 128-bits (composed of 4 packed single-precision (32-bit) floating-point elements) from this vector into unaligned memory.
	void CopyToUnalignedData(float* _Dst) const
	{
		_mm_storeu_ps(_Dst, Data);
	}

	//Store 128-bits (composed of 4 packed single-precision (32-bit) floating-point elements) from this vector into aligned memory.
	void CopyToAlignedData(float* _Dst) const
	{
		_mm_store_ps(_Dst, Data);
	}

	//Shuffle single-precision (32-bit) floating-point elements in a using the control in imm8, and store the results in dst.
	template<const uint32 _a, const uint32 _b, const uint32 _c, const uint32 _d>
	[[nodiscard]] static INLINE float4 Shuffle(const float4& _A, const float4& _B)
	{
		return _mm_shuffle_ps(_A.Data, _B.Data, _MM_SHUFFLE(_a, _b, _c, _d));
	}

	INLINE static float4 Abs(const float4& _A)
	{
		return _mm_andnot_ps(_A.Data, float4(1.0f, 1.0f, 1.0f, 1.0f));
	}

	INLINE float4 HorizontalAdd(const float4& _Other) const
	{
		return _mm_hadd_ps(Data, _Other);
	}

	//Horizontally add adjacent pairs of single - precision(32 - bit) floating - point elements in a and b, and pack the results in dst.
	[[nodiscard]] INLINE float4 HorizontalSub(const float4& _Other) const
	{
		return _mm_hsub_ps(Data, _Other.Data);
	}

	//Alternatively add and subtract packed single-precision (32-bit) floating-point elements in a to/from packed elements in b, and store the results in dst
	[[nodiscard]] INLINE float4 Add13Sub02(const float4& _Other) const
	{
		return _mm_addsub_ps(Data, _Other.Data);
	}

	//Conditionally multiply the packed single-precision (32-bit) floating-point elements in a and b using the high 4 bits in imm8, sum the four products, and conditionally store the sum in dst using the low 4 bits of imm8.
	[[nodiscard]] INLINE static float4 DotProduct(const float4& _A, const float4& _B)
	{
		return _mm_dp_ps(_A.Data, _B.Data, 0xff);
	}

	[[nodiscard]] INLINE float4 SquareRoot(const float4& _Other) const
	{
		return _mm_sqrt_ps(Data);
	}

	//Compute the square root of the lower single-precision (32-bit) floating-point element in a, store the result in the lower element of dst, and copy the upper 3 packed elements from a to the upper elements of dst.
	[[nodiscard]] INLINE float4 SquareRootToLower(const float4& _Other) const
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


	INLINE __m128i Toint4() const
	{
		return _mm_cvtps_epi32(Data);
	}


	INLINE float4 operator+(const float4& _Other) const
	{
		return _mm_add_ps(Data, _Other.Data);
	}

	INLINE float4& operator+=(const float4& _Other)
	{
		Data = _mm_add_ps(Data, _Other.Data);
		return *this;
	}

	INLINE float4 operator-(const float4& _Other) const
	{
		return _mm_sub_ps(Data, _Other.Data);
	}

	INLINE float4& operator-=(const float4& _Other)
	{
		Data = _mm_sub_ps(Data, _Other.Data);
		return *this;
	}

	INLINE float4 operator*(const float4& _Other) const
	{
		return _mm_mul_ps(Data, _Other.Data);
	}

	INLINE float4& operator*=(const float4& _Other)
	{
		Data = _mm_mul_ps(Data, _Other.Data);
		return *this;
	}

	INLINE float4 operator/(const float4& _Other) const
	{
		return _mm_div_ps(Data, _Other.Data);
	}

	INLINE float4& operator/=(const float4& _Other)
	{
		Data = _mm_div_ps(Data, _Other.Data);
		return *this;
	}


	INLINE float4 operator==(const float4& _Other) const
	{
		return _mm_cmpeq_ps(Data, _Other.Data);
	}

	INLINE float4 operator!=(const float4& _Other) const
	{
		return _mm_cmpneq_ps(Data, _Other.Data);
	}

	INLINE float4 operator>(const float4& _Other) const
	{
		return _mm_cmpgt_ps(Data, _Other.Data);
	}

	INLINE float4 operator>=(const float4& _Other) const
	{
		return _mm_cmpge_ps(Data, _Other.Data);
	}

	INLINE float4 operator<(const float4& _Other) const
	{
		return _mm_cmplt_ps(Data, _Other.Data);
	}

	INLINE float4 operator<=(const float4& _Other) const
	{
		return _mm_cmple_ps(Data, _Other.Data);
	}

	INLINE float4 operator&(const float4& _Other) const
	{
		return _mm_and_ps(Data, _Other.Data);
	}

	INLINE float4 operator|(const float4& _Other) const
	{
		return _mm_or_ps(Data, _Other.Data);
	}
};