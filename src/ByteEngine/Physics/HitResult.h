#pragma once

#include <GTSL/Math/Vectors.h>
#include <ByteEngine/Core.h>

struct HitResult {
	/**
	 * \brief Defines whether there was hit or not.\n
	 * true = there was a collision.\n
	 * false = there was no collision.
	 */
	bool WasHit = false;

	/**
	 * \brief Defines the position (in world space) of the hit.
	 */
	GTSL::Vector3 Position;

	GTSL::Vector3 PointOnA;

	GTSL::Vector3 PointOnB;

	/**
	 * \brief Defines the normal (in world space) of the hit.
	 */
	GTSL::Vector3 Normal;

	/**
	 * \brief Defines the penetration distance of the two colliding bodies. This is along the HitNormal.
	 */
	float32 T = 0;
};
