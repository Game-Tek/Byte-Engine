#include "Vector3.h"

#include "GSM.hpp"

Vector3::Vector3(const Rotator& rotator) : X(GSM::Cosine(rotator.X) * GSM::Sine(rotator.Y)), Y(GSM::Sine(rotator.X)), Z(GSM::Cosine(rotator.X) * GSM::Cosine(rotator.Y))
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
    *this = u * 2.0f * GSM::DotProduct(u, *this) + (s * s - GSM::DotProduct(u, u)) * *this + 2.0f * s * GSM::Cross(u, *this);

    return *this;
}
