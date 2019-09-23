#pragma once

#pragma once

#define MATRIX_SIZE 16

#include "Core.h"

#include "Vector4.h"

//Index increases in row order.

//Used to create 4x4 matrices with floating point precision.
class GS_API Matrix4
{
public:
	Matrix4() : Array{ 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 }
	{
	}

	Matrix4(const float A, const float B, const float C, const float D, const float E, const float F, const float G, const float H, const float I, const float J, const float K, const float L, const float M, const float N, const float O, const float P) :
		Array{ A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P }
	{
	}

	~Matrix4() = default;

	void Identity()
	{
		Array[0] = 1.0f;
		Array[5] = 1.0f;
		Array[10] = 1.0f;
		Array[15] = 1.0f;

		return;
	}

	const float * GetData() const { return Array; }

	Matrix4 operator+ (const float Other) const
	{
		Matrix4 Result;

		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] += Other;
		}

		return Result;
	}

	Matrix4 operator+ (const Matrix4 & Other) const
	{
		Matrix4 Result;

		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] += Other[i];
		}

		return Result;
	}

	Matrix4 & operator+= (const float Other)
	{
		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] += Other;
		}

		return *this;
	}

	Matrix4 & operator+= (const Matrix4 & Other)
	{
		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] += Other[i];
		}

		return *this;
	}

	Matrix4 operator- (const float Other) const
	{
		Matrix4 Result;

		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] -= Other;
		}

		return Result;
	}

	Matrix4 operator- (const Matrix4 & Other) const
	{
		Matrix4 Result;

		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] -= Other[i];
		}

		return Result;
	}

	Matrix4 & operator-= (const float Other)
	{
		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] -= Other;
		}

		return *this;
	}

	Matrix4 & operator-= (const Matrix4 & Other)
	{
		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] -= Other[i];
		}

		return *this;
	}

	Matrix4 operator* (const float Other) const
	{
		Matrix4 Result;

		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] *= Other;
		}

		return Result;
	}

	Vector4 operator* (const Vector4 & Other) const
	{
		Vector4 Result;

		Result.X = Array[0] * Other.X + Array[1] * Other.X + Array[2] * Other.X + Array[3] * Other.X;
		Result.Y = Array[4] * Other.Y + Array[5] * Other.Y + Array[6] * Other.Y+ Array[7] * Other.Y;
		Result.Z = Array[8] * Other.Z + Array[9] * Other.Z + Array[10] * Other.Z + Array[11] * Other.Z;
		Result.W = Array[12] * Other.W + Array[13] * Other.W + Array[14] * Other.W + Array[15] * Other.W;

		return Result;
	}

	Matrix4 operator* (const Matrix4 & Other) const
	{
		Matrix4 Result;

		for (uint8 y = 0; y < 4; y++)
		{
			for (uint8 x = 0; x < 4; x++)
			{
				float Sum = 0.0f;

				for (uint8 e = 0; e < 4; e++)
				{
					Sum += Result[e + y * 4] * Other[x + e * 4];
				}

				Result[x + y * 4] = Sum;
			}
		}

		return Result;
	}

	Matrix4 & operator*= (const float Other)
	{
		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] *= Other;
		}

		return *this;
	}

	Matrix4 & operator*= (const Matrix4 & Other)
	{
		for (uint8 y = 0; y < 4; y++)
		{
			for (uint8 x = 0; x < 4; x++)
			{
				float Sum = 0.0f;

				for (uint8 e = 0; e < 4; e++)
				{
					Sum += Array[e + y * 4] * Other[x + e * 4];
				}

				Array[x + y * 4] = Sum;
			}
		}

		return *this;
	}

	Matrix4 operator/ (const float Other) const
	{
		Matrix4 Result;

		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] /= Other;
		}

		return Result;
	}

	Matrix4 & operator/= (const float Other)
	{
		for (uint8 i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] /= Other;
		}

		return *this;
	}

	float & operator[] (const uint8 Index) { return Array[Index]; }

	float operator[] (const uint8 Index) const { return Array[Index]; }

private:
	float Array[MATRIX_SIZE];
};