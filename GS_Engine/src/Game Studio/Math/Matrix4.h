#pragma once

#pragma once

#define MATRIX_SIZE 16

#include "Core.h"

#include "Vector4.h"

//Index increases in row order.

/**
 * \brief Defines a 4x4 matrix with floating point precision.\n
 * Data is stored in row major order.
 * E.J:\n
 * 
 * Matrix:\n
 * A B C D\n
 * E F G H\n
 * I J K L\n
 * M N O P\n
 *
 * Array(data): A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P\n
 *
 * Most operations are accelerated by SIMD code.
 * 
 */
class GS_API Matrix4
{
public:
	/**
	 * \brief Default constructor. Sets all of the matrices' components as 0.
	 */
	Matrix4() : Array{ 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 }
	{
	}

	/**
	 * \brief Builds an identity matrix with _A being each of the identity elements value.
	 * Usually one(1) will be used.
	 * \n
	 * \n
	 * _A 0 0 0\n
	 * 0 _A 0 0\n
	 * 0 0 _A 0\n
	 * 0 0 0 _A\n
	 * 
	 * \param _A float to set each of the matrix identity elements value as.
	 */
	explicit Matrix4(const float _A) : Array{ _A, 0, 0, 0, 0, _A, 0, 0, 0, 0, _A, 0, 0, 0, 0, _A }
	{
	}
	
	/**
	 * \brief Constructs the matrix with every component set as the corresponding parameter.
	 * \param _Row0_Column0 float to set the matrices' Row0_Column0 component as.
	 * \param _Row0_Column1 float to set the matrices' Row0_Column1	component as.
	 * \param _Row0_Column2 float to set the matrices' Row0_Column2	component as.
	 * \param _Row0_Column3 float to set the matrices' Row0_Column3	component as.
	 * \param _Row1_Column0 float to set the matrices' Row1_Column0	component as.
	 * \param _Row1_Column1 float to set the matrices' Row1_Column1	component as.
	 * \param _Row1_Column2 float to set the matrices' Row1_Column2	component as.
	 * \param _Row1_Column3 float to set the matrices' Row1_Column3	component as.
	 * \param _Row2_Column0 float to set the matrices' Row2_Column0	component as.
	 * \param _Row2_Column1 float to set the matrices' Row2_Column1	component as.
	 * \param _Row2_Column2 float to set the matrices' Row2_Column2	component as.
	 * \param _Row2_Column3 float to set the matrices' Row2_Column3	component as.
	 * \param _Row3_Column0 float to set the matrices' Row3_Column0	component as.
	 * \param _Row3_Column1 float to set the matrices' Row3_Column1	component as.
	 * \param _Row3_Column2 float to set the matrices' Row3_Column2	component as.
	 * \param _Row3_Column3 float to set the matrices' Row3_Column3	component as.
	 */
	Matrix4(const float _Row0_Column0, const float _Row0_Column1, const float _Row0_Column2, const float _Row0_Column3,
			const float _Row1_Column0, const float _Row1_Column1, const float _Row1_Column2, const float _Row1_Column3,
			const float _Row2_Column0, const float _Row2_Column1, const float _Row2_Column2, const float _Row2_Column3,
			const float _Row3_Column0, const float _Row3_Column1, const float _Row3_Column2, const float _Row3_Column3) :
			Array{ _Row0_Column0, _Row0_Column1, _Row0_Column2, _Row0_Column3,
				   _Row1_Column0, _Row1_Column1, _Row1_Column2, _Row1_Column3,
				   _Row2_Column0, _Row2_Column1, _Row2_Column2, _Row2_Column3,
				   _Row3_Column0, _Row3_Column1, _Row3_Column2, _Row3_Column3 }
	{
	}

	~Matrix4() = default;

	/**
	 * \brief Sets all of this matrices' components to represent an Identity matrix.\n
	 *
	 * 1 0 0 0\n
	 * 0 1 0 0\n
	 * 0 0 1 0\n
	 * 0 0 0 1\n
	 */
	void MakeIdentity()
	{
		for (auto& element : Array)
		{
			element = 0.0f;
		}
		
		Array[0] = 1.0f;
		Array[5] = 1.0f;
		Array[10] = 1.0f;
		Array[15] = 1.0f;

		return;
	}

	//Matrix4& operator=(float _Array[16]) { Array = _Array; return *this; }
	//
	//void SetData(float _Array[]) { Array = _Array; }
	
	/**
	 * \brief Returns a pointer to the matrices' data array.
	 * \return const float* to the matrices' data.
	 */
	[[nodiscard]] const float* GetData() const { return Array; }

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

	Vector4 operator* (const Vector4& Other) const;
	//{
	//	Vector4 Result;
	//
	//	Result.X = Array[0] * Other.X + Array[1] * Other.X + Array[2] * Other.X + Array[3] * Other.X;
	//	Result.Y = Array[4] * Other.Y + Array[5] * Other.Y + Array[6] * Other.Y+ Array[7] * Other.Y;
	//	Result.Z = Array[8] * Other.Z + Array[9] * Other.Z + Array[10] * Other.Z + Array[11] * Other.Z;
	//	Result.W = Array[12] * Other.W + Array[13] * Other.W + Array[14] * Other.W + Array[15] * Other.W;
	//
	//	return Result;
	//}

	Matrix4 operator* (const Matrix4& Other) const;

	//Matrix4 operator* (const Matrix4& Other) const
	//{
	//	Matrix4 Result(1);
	//
	//	for (int i = 0; i < 4; i++)
	//	{
	//		for (int j = 0; j < 4; j++)
	//		{
	//			Result[i + j] = 0;
	//			for (int k = 0; k < 4; k++)
	//			{
	//				Result[i + j] += Array[i + k] * Other[k + j];
	//			}
	//		}
	//	}
	//
	//	return Result;
	//}

	Matrix4& operator*= (const float Other);
	//{
	//	for (uint8 i = 0; i < MATRIX_SIZE; i++)
	//	{
	//		Array[i] *= Other;
	//	}
	//
	//	return *this;
	//}

	Matrix4& operator*= (const Matrix4& Other);
	//{
	//	for (uint8 y = 0; y < 4; y++)
	//	{
	//		for (uint8 x = 0; x < 4; x++)
	//		{
	//			float Sum = 0.0f;
	//
	//			for (uint8 e = 0; e < 4; e++)
	//			{
	//				Sum += Array[e + y * 4] * Other[x + e * 4];
	//			}
	//
	//			Array[x + y * 4] = Sum;
	//		}
	//	}
	//
	//	return *this;
	//}

	float& operator[] (const uint8 Index) { return Array[Index]; }
	float operator[] (const uint8 Index) const { return Array[Index]; }

	float operator() (const uint8 _Row, const uint8 _Column) const { return Array[_Row * 4 + (_Column - 1)]; }
	float& operator() (const uint8 _Row, const uint8 _Column) { return Array[_Row * 4 + (_Column - 1)]; }
	
private:
	float Array[MATRIX_SIZE];
};