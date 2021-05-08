#pragma once

#include "Utility/Shapes/Box.h"

#include "Math/Vectors.h"

class ReverbVolume
{
	/**
	 * \brief Defines the space this reverb volume takes up.
	 */
	Box extent;

	void (*decayFunction)(float&, const Vector3&) = nullptr;
};
