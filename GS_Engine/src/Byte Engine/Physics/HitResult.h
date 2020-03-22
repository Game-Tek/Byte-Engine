#pragma once
#include "Math/Vector3.h"
#include "Containers/Pair.h"

struct HitData
{
	/**
	 * \brief Defines whether there was hit or not.\n
	 * true = there was a collision.\n
	 * false = there was no collision.
	 */
	bool WasHit = false;

	/**
	 * \brief Defines the position (in world space) of the hit.
	 */
	Vector3 HitPosition;

	/**
	 * \brief Defines the normal (in world space) of the hit.
	 */
	Vector3 HitNormal;

	/**
	 * \brief Defines the penetration distance of the two colliding bodies. This is along the HitNormal.
	 */
	float PenetrationDistance = 0;

	/**
	 * \brief Defines a pair of pointers to the two colliding bodies.
	 */
	Pair<void*, void*> HitObjects;
};
