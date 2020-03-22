#include "Vector3.h"

#include "BEM.hpp"

Vector3::Vector3(const Rotator& rotator) : X(BEM::Cosine(rotator.X) * BEM::Sine(rotator.Y)), Y(BEM::Sine(rotator.X)),
                                           Z(BEM::Cosine(rotator.X) * BEM::Cosine(rotator.Y))
{
	//CODE IS CORRECT
}

Vector3 operator*(const float& lhs, const Vector3& rhs)
{
	return Vector3(rhs.X * lhs, rhs.Y * lhs, rhs.Z * lhs);
}

Vector3& Vector3::operator*=(const Quaternion& quaternion)
{
	// Extract the vector part of the quaternion
	Vector3 u(quaternion.X, quaternion.Y, quaternion.Z);

	// Extract the scalar part of the quaternion
	float s = quaternion.Q;

	// Do the math
	*this = u * 2.0f * BEM::DotProduct(u, *this) + (s * s - BEM::DotProduct(u, u)) * *this + 2.0f * s * BEM::
		Cross(u, *this);

	return *this;
}
