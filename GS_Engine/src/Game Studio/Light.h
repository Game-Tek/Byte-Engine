#pragma once

#include "Core.h"

#include "WorldObject.h"

#include "RGB.h"

GS_CLASS Light : public WorldObject
{
public:
	Light();
	~Light();

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
	RGB Color;
};

