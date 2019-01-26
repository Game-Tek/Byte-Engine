#pragma once

#pragma once

#define MATRIX_SIZE 16

#include "Core.h"

#include "Vector4.h"

//Index increases in row order.

//Used to create 4x4 matrices with floating point precision.
GS_CLASS Matrix4
{
public:

	Matrix4()
	{
		for (unsigned short i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] = 0;
		}
	}

	Matrix4(float A, float B, float C, float D, float E, float F, float G, float H, float I, float J, float K, float L, float M, float N, float O, float P)
	{
		Array[0] = A;
		Array[1] = B;
		Array[2] = C;
		Array[3] = D;
		Array[4] = E;
		Array[5] = F;
		Array[6] = G;
		Array[7] = H;
		Array[8] = I;
		Array[9] = J;
		Array[10] = K;
		Array[11] = L;
		Array[12] = M;
		Array[13] = N;
		Array[14] = O;
		Array[15] = P;
	}

	~Matrix4()
	{
	}

	void Identity()
	{
		Array[0] = 1;
		Array[5] = 1;
		Array[10] = 1;
		Array[15] = 1;

		return;
	}

	const float * GetData() const { return Array; }

	Matrix4 operator+ (float Other) const
	{
		Matrix4 Result;

		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] += Other;
		}

		return Result;
	}

	Matrix4 operator+ (const Matrix4 & Other) const
	{
		Matrix4 Result;

		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] += Other[i];
		}

		return Result;
	}

	Matrix4 & operator+= (float Other)
	{
		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] += Other;
		}

		return *this;
	}

	Matrix4 & operator+= (const Matrix4 & Other)
	{
		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] += Other[i];
		}

		return *this;
	}

	Matrix4 operator- (float Other) const
	{
		Matrix4 Result;

		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] -= Other;
		}

		return Result;
	}

	Matrix4 operator- (const Matrix4 & Other) const
	{
		Matrix4 Result;

		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] -= Other[i];
		}

		return Result;
	}

	Matrix4 & operator-= (float Other)
	{
		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] -= Other;
		}

		return *this;
	}

	Matrix4 & operator-= (const Matrix4 & Other)
	{
		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] -= Other[i];
		}

		return *this;
	}

	Matrix4 operator* (float Other) const
	{
		Matrix4 Result;

		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
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

		for (unsigned short y = 0; y < 4; y++)
		{
			for (unsigned short x = 0; x < 4; x++)
			{
				float Sum = 0.0f;
				for (unsigned short e = 0; e < 4; e++)
				{
					Sum += Result[e + y * 4] * Other[x + e * 4];
				}

				Result[x + y * 4] = Sum;
			}
		}

		return Result;
	}

	Matrix4 & operator*= (float Other)
	{
		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] *= Other;
		}

		return *this;
	}

	Matrix4 & operator*= (const Matrix4 & Other)
	{
		for (unsigned short y = 0; y < 4; y++)
		{
			for (unsigned short x = 0; x < 4; x++)
			{
				float Sum = 0.0f;
				for (unsigned short e = 0; e < 4; e++)
				{
					Sum += Array[e + y * 4] * Other[x + e * 4];
				}

				Array[x + y * 4] = Sum;
			}
		}

		return *this;
	}

	Matrix4 operator/ (float Other) const
	{
		Matrix4 Result;

		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Result[i] /= Other;
		}

		return Result;
	}

	Matrix4 & operator/= (float Other)
	{
		for (unsigned char i = 0; i < MATRIX_SIZE; i++)
		{
			Array[i] /= Other;
		}

		return *this;
	}

	float & operator[] (unsigned int Index) { return Array[Index]; }

	float operator[] (unsigned int Index) const { return Array[Index]; }

private:
	float Array[MATRIX_SIZE];
};