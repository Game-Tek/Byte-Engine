#pragma once

struct ForceInstructions
{
};

class ForceGenerator
{
	/**
	* \brief Determines the intensity at which this magnet force generator pushes or pulls objects. When positive it will repel objects, when positive it will attract them.
	*/
	float intensity = 0;
	
public:
	virtual const char* GetForceType() = 0;
	virtual ForceInstructions GetForceInstructions();

	auto& GetIntensity() { return intensity; }
};
