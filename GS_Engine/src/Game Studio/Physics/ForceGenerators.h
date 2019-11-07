#pragma once

#include "ForceGenerator.h"

#include "Math/Quaternion.h"

#include "Utility/Shapes/SphereWithFallof.h"
#include "Utility/Shapes/BoxWithFalloff.h"
#include "Utility/Shapes/ConeWithFalloff.h"

class ExplosionGenerator : public ForceGenerator
{
	SphereWithFalloff effectVolume = 0;

public:
	const char* GetForceType() override { return "Explosion"; }
};

class BuoyancyGenerator : public ForceGenerator
{
	/**
	 * \brief Fluid weight(KG) per cubic meter. E.I: water is 1000kg.
	 */
	float fluidWeight = 1000;

	Box effectVolume;
public:
	const char* GetForceType() override { return "Buoyancy"; }
};

class MagnetGenerator : public ForceGenerator
{
	SphereWithFalloff effectVolume;
	
public:
	const char* GetForceType() override { return "Magnet"; }
};

class WindGenerator : public ForceGenerator
{
	Vector3 windDirection;
	BoxWithFalloff effectVolume;
	
public:
	const char* GetForceType() override { return "Wind"; }
};

class DirectionalWindGenerator : public ForceGenerator
{
	Quaternion windOrientation;
	ConeWithFalloff windDirection;
	
public:
	const char* GetForceType() override { return "Directional Wind"; }
};