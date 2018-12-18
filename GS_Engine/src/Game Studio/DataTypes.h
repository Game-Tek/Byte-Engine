#pragma once

//Used to specify a location in 2D space with floating point precision.
struct Vector2
{
	float X;
	float Y;

	Vector2 operator+ (const Vector2 & Vec1)
	{
		return { X + Vec1.X, Y + Vec1.Y };
	}

	Vector2 operator- (const Vector2 & Vec1)
	{
		return { X - Vec1.X, Y - Vec1.Y };
	}

	Vector2 operator* (float A)
	{
		return { X * A, Y * A };
	}

	Vector2 operator/ (float A)
	{
		return { X / A, Y / A };
	}
};

//Used to specify a location in 3D space with floating point precision.
struct Vector3
{
	float X;
	float Y;
	float Z;

	Vector3 operator+ (const Vector3 & Vec1)
	{
		return { X + Vec1.X, Y + Vec1.Y, Z + Vec1.Z };
	}

	Vector3 operator- (const Vector3 & Vec1)
	{
		return { X - Vec1.X, Y - Vec1.Y, Z - Vec1.Z };
	}

	Vector3 operator* (float A)
	{
		return { X * A, Y * A, Z * A };
	}

	Vector3 operator/ (float A)
	{
		return { X / A, Y / A, Z / A };
	}

	bool operator== (const Vector3 & Vec1)
	{
		return X == Vec1.X && Y == Vec1.Y && Z == Vec1.Z;
	}

	bool operator!= (const Vector3 & Vec1)
	{
		return X != Vec1.X && Y != Vec1.Y && Z != Vec1.Z;
	}
};

//Used to specify a rotation with floating point precision.
struct Rotator
{
	float Pitch;	
	float Yaw;
	float Roll;
};

//Used to specify a location in 3D space with floating point precision.
struct Quat
{
	float X;
	float Y;
	float Z;
	float Q;
};

//Used to specify a color in RGBA.
struct Color
{
	int R;
	int G;
	int B;
	int A;
};

//Used to specify the transform of an object in 2D space.
struct Transform2
{
	Vector2 Location;
	float Angle;
	Vector2 Size;
};

//Used to specify the transform of an object in 3D space.
struct Transform3
{
	Vector3 Location;
	Rotator Rotation;
	Vector3 Size;
};