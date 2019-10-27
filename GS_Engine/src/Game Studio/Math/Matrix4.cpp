#include "Matrix4.h"

#include "SIMD/float4.h"

Vector4 Matrix4::operator*(const Vector4& Other) const
{
	alignas(16) Vector4 Result;

	const float4 P1(float4(&Other.X) * float4(&Array[0]));
	const float4 P2(float4(&Other.Y) * float4(&Array[4]));
	const float4 P3(float4(&Other.Z) * float4(&Array[8]));
	const float4 P4(float4(&Other.W) * float4(&Array[12]));

	const float4 res = P1 + P2 + P3 + P4;

	res.CopyToAlignedData(&Result.X);

	return Result;
}

Matrix4 Matrix4::operator*(const Matrix4& Other) const
{
	Matrix4 Result;

	float4 Row1(&Other.Array[0]);
	float4 Row2(&Other.Array[4]);
	float4 Row3(&Other.Array[8]);
	float4 Row4(&Other.Array[12]);

	float4 Bro1;
	float4 Bro2;
	float4 Bro3;
	float4 Bro4;

	float4 Row;

	for (uint8 i = 0; i < 4; ++i)
	{
		Bro1 = &Array[4 * i + 0];
		Bro2 = &Array[4 * i + 1];
		Bro3 = &Array[4 * i + 2];
		Bro4 = &Array[4 * i + 3];

		Row = (Bro1 * Row1) + (Bro2 * Row2) + (Bro3 * Row3) + (Bro4 * Row4);

		Row.CopyToAlignedData(&Result.Array[4 * i]);
	}

	return Result;
}

Matrix4& Matrix4::operator*=(const float Other)
{
	float Input = Other;
	const __m512 InputVector = _mm512_set1_ps(Input);
	const __m512 MatrixVector = _mm512_load_ps(Array);

	const __m512 Result = _mm512_mul_ps(InputVector, MatrixVector);

	_mm512_store_ps(Array, Result);

	return *this;
}

Matrix4& Matrix4::operator*=(const Matrix4& Other)
{
	float4 Row1(&Other.Array[0]);
	float4 Row2(&Other.Array[4]);
	float4 Row3(&Other.Array[8]);
	float4 Row4(&Other.Array[12]);

	float4 Bro1;
	float4 Bro2;
	float4 Bro3;
	float4 Bro4;

	float4 Row;

	for (uint8 i = 0; i < 4; ++i)
	{
		Bro1 = &Array[4 * i + 0];
		Bro2 = &Array[4 * i + 1];
		Bro3 = &Array[4 * i + 2];
		Bro4 = &Array[4 * i + 3];

		Row = (Bro1 * Row1) + (Bro2 * Row2) + (Bro3 * Row3) + (Bro4 * Row4);

		Row.CopyToAlignedData(&Array[4 * i]);
	}

	return *this;
}
