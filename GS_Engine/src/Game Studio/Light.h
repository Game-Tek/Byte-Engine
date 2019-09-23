#pragma once

#include "Core.h"

#include "Game/WorldObject.h"
#include "Utility/RGB.h"

class GS_API Light : public WorldObject
{
public:
	Light() = default;
	~Light() = default;

	//Returns the value of lumens for this light.
	float GetLumens() const { return Lumens; }
	//Returns the color for this light.
	RGB GetRGB() const { return Color; }

	//Sets Lumens as NewLumens.
	void SetLumens(const float NewLumens) { Lumens = NewLumens; }
	//Sets Color as NewColor.
	void SetColor(const RGB & NewColor) { Color = NewColor; }
	//Sets Color from a color temperature.
	void SetColor(const uint16 ColorTemperature);

protected:
	//Determines the intensity of the light in lumens.
	float Lumens = 1000.0f;
	RGB Color = { 0.0f , 0.0f , 0.0f };
};

