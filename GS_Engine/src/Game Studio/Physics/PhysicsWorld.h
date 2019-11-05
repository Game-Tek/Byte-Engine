#pragma once

#include "Math/Vector3.h"

class PhysicsWorld
{
	/**
	 * \brief Specifies the gravity acceleration of this world. Is in Meters/Seconds.
	 * Usual value will be X = 0, Y = -10, Z = 0.
	 */
	Vector3 gravity{0, -10, 0};

	/**
	 * \brief Specifies how much speed the air resistance removes from entities.\n
	 * Default value is 0.0001.
	 */
	float airDensity = 0.001;
};
