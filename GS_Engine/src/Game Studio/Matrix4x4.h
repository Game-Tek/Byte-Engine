#pragma once

#include "Core.h"

//Used to create 4x4 matrices with floating point precision.
GS_CLASS Matrix4x4
{
public:
	Matrix4x4();
	~Matrix4x4();

	void Identity();

	Matrix4x4 operator* (const Matrix4x4 & Other);
	float & operator[] (unsigned int Index) { return Array[Index]; }
private:
	float Array[4 * 4];
};

Matrix4x4::Matrix4x4()
{
	for (unsigned short i = 0; i < 16; i++)
	{
		Array[i] = 0;
	}
}

Matrix4x4::~Matrix4x4()
{
}

void Matrix4x4::Identity()
{
	Array[0] = 1;
	Array[5] = 1;
	Array[10] = 1;
	Array[15] = 1;

	return;
}

Matrix4x4 Matrix4x4::operator* (const Matrix4x4 & Other)
{
	Matrix4x4 Result;

	for (unsigned short y = 0; y < 4; y++)
	{
		for (unsigned short x = 0; x < 4; x++)
		{
			float Sum = 0.0f;
			for (unsigned short e = 0; e < 4; e++)
			{
				Sum += Result.Array[e + y * 4] * Other.Array[x + e * 4];
			}

			Result.Array[x + y * 4] = Sum;
		}
	}

	return Result;
}