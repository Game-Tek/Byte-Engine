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
	
	Vector3(const Vector3& _Other) = default;

	~Vector3() = default;

	Vector3 operator+ (float _Other) const
	{
		return { X + _Other, Y + _Other, Z + _Other };
	}

	Vector3 operator+ (const Vector3 & _Other) const
	{
		return { X + _Other.X, Y + _Other.Y, Z + _Other.Z };
	}

	Vector3 & operator+= (float _Other)
	{
		X += _Other;
		Y += _Other;
		Z += _Other;

		return *this;
	}

	Vector3 & operator+= (const Vector3 & _Other)
	{
		X += _Other.X;
		Y += _Other.Y;
		Z += _Other.Z;

		return *this;
	}

	Vector3 operator- (float _Other) const
	{
		return { X - _Other, Y - _Other, Z - _Other };
	}

	Vector3 operator- (const Vector3 & _Other) const
	{
		return { X - _Other.X, Y - _Other.Y, Z - _Other.Z };
	}

	Vector3 & operator-= (float _Other)
	{
		X -= _Other;
		Y -= _Other;
		Z -= _Other;

		return *this;
	}

	Vector3 & operator-= (const Vector3 & _Other)
	{
		X -= _Other.X;
		Y -= _Other.Y;
		Z -= _Other.Z;

		return *this;
	}

	Vector3 operator* (float _Other) const
	{
		return { X * _Other, Y * _Other, Z * _Other };
	}

	Vector3 operator* (const Vector3& _Other) const
	{
		return { X * _Other.X, Y * _Other.Y, Z * _Other.Z };
	}

	Vector3 & operator*= (float _Other)
	{
		X *= _Other;
		Y *= _Other;
		Z *= _Other;

		return *this;
	}

	Vector3 operator/ (float _Other) const
	{
		return { X / _Other, Y / _Other, Z / _Other };
	}

	Vector3 & operator/= (float _Other)
	{
		X /= _Other;
		Y /= _Other;
		Z /= _Other;

		return *this;
	}

	inline bool operator== (const Vector3 & _Other)
	{
		return X == _Other.X && Y == _Other.Y && Z == _Other.Z;
	}

	inline bool operator!= (const Vector3 & _Other)
	{
		return X != _Other.X || Y != _Other.Y || Z != _Other.Z;
	}
};