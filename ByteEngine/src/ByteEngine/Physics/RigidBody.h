#pragma once

class RigidBody
{
	/**
	 * \brief Specifies the inverse mass of this body.
	 */
	float inverseBodyMass = 1;

public:
	void SetMass(const float _Mass) { inverseBodyMass = 1 / _Mass; }
};
