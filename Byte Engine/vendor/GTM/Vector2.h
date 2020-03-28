#pragma once

//Used to specify a location in 2D space with floating point precision.
class Vector2
{
public:
	//X component of this vector.
	float X = 0.0f;

	//Y component of this vector.
	float Y = 0.0f;

	Vector2() = default;

	constexpr Vector2(float X, float Y) : X(X), Y(Y)
	{
	}

	Vector2(const Vector2& Other) : X(Other.X), Y(Other.Y)
	{
	}

	~Vector2() = default;

	Vector2 operator+(float Other) const
	{
		return {X + Other, Y + Other};
	}

	Vector2 operator+(const Vector2& Other) const
	{
		return {X + Other.X, Y + Other.Y};
	}

	Vector2& operator+=(float Other)
	{
		X += Other;
		Y += Other;

		return *this;
	}

	Vector2& operator+=(const Vector2& Other)
	{
		X += Other.X;
		Y += Other.Y;

		return *this;
	}

	Vector2 operator-(float Other) const
	{
		return {X - Other, Y - Other};
	}

	Vector2 operator-(const Vector2& Other) const
	{
		return {X - Other.X, Y - Other.Y};
	}

	Vector2& operator-=(float Other)
	{
		X -= Other;
		Y -= Other;

		return *this;
	}

	Vector2& operator-=(const Vector2& Other)
	{
		X -= Other.X;
		Y -= Other.Y;

		return *this;
	}

	Vector2 operator*(float Other) const
	{
		return {X * Other, Y * Other};
	}

	Vector2& operator*=(float Other)
	{
		X *= Other;
		Y *= Other;

		return *this;
	}

	Vector2 operator/(float Other) const
	{
		return {X / Other, Y / Other};
	}

	Vector2& operator/=(float Other)
	{
		X /= Other;
		Y /= Other;

		return *this;
	}

	inline bool operator==(const Vector2& Other)
	{
		return X == Other.X && Y == Other.Y;
	}

	inline bool operator!=(const Vector2& Other)
	{
		return X != Other.X || Y != Other.Y;
	}
};
