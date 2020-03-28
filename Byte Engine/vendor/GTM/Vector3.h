#pragma once

//Used to specify a location in 3D space with floating point precision.
class Vector3
{
public:
	//X component of this vector.
	float X = 0.0f;

	//Y component of this vector.
	float Y = 0.0f;

	//Z component of this vector.
	float Z = 0.0f;


	Vector3() = default;

	Vector3(float X, float Y, float Z) : X(X), Y(Y), Z(Z)
	{
	}

	explicit Vector3(const class Rotator& rotator);

	Vector3(const Vector3& other) = default;

	~Vector3() = default;

	Vector3 operator+(float other) const
	{
		return {X + other, Y + other, Z + other};
	}

	Vector3 operator+(const Vector3& other) const
	{
		return {X + other.X, Y + other.Y, Z + other.Z};
	}

	Vector3& operator+=(float other)
	{
		X += other;
		Y += other;
		Z += other;

		return *this;
	}

	Vector3& operator+=(const Vector3& other)
	{
		X += other.X;
		Y += other.Y;
		Z += other.Z;

		return *this;
	}

	Vector3 operator-(float other) const
	{
		return {X - other, Y - other, Z - other};
	}

	Vector3 operator-(const Vector3& other) const
	{
		return {X - other.X, Y - other.Y, Z - other.Z};
	}

	Vector3& operator-=(float other)
	{
		X -= other;
		Y -= other;
		Z -= other;

		return *this;
	}

	Vector3& operator-=(const Vector3& other)
	{
		X -= other.X;
		Y -= other.Y;
		Z -= other.Z;

		return *this;
	}

	Vector3 operator*(float other) const
	{
		return {X * other, Y * other, Z * other};
	}

	Vector3 operator*(const Vector3& other) const
	{
		return {X * other.X, Y * other.Y, Z * other.Z};
	}

	Vector3& operator*=(float other)
	{
		X *= other;
		Y *= other;
		Z *= other;

		return *this;
	}

	Vector3& operator*=(const class Quaternion& quaternion);

	Vector3 operator/(float other) const
	{
		return {X / other, Y / other, Z / other};
	}

	Vector3& operator/=(float other)
	{
		X /= other;
		Y /= other;
		Z /= other;

		return *this;
	}

	inline bool operator==(const Vector3& other)
	{
		return X == other.X && Y == other.Y && Z == other.Z;
	}

	inline bool operator!=(const Vector3& other)
	{
		return X != other.X || Y != other.Y || Z != other.Z;
	}
};
