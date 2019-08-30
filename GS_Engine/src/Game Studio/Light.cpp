#include "Light.h"

/*
void Light::SetColor(const uint16 ColorTemperature)
{
	//CODE BY TANNER HELLAND
	//http://www.tannerhelland.com/4435/convert-temperature-rgb-algorithm-code/

	const uint16 Temperature_ = ColorTemperature / 100;

	//----------RED----------
	if (Temperature_ <= 66)
	{
		Color.R = 255;
	}
	else
	{
		Color.R = Temperature_ - 60;
		Color.R = 329.698727446 * (Color.R ^ -0.1332047592);

		if (Color.R < 0)
		{
			Color.R = 0;
		}
		if (Color.R > 255)
		{
			Color.R = 255;
		}
	}
	//----------RED----------

	//----------GREEN----------
	if (Temperature_ <= 66)
	{
		Color.G = Temperature_;
		Color.G = 99.4708025861 * Ln(Color.G) - 161.1195681661;
		if (Color.G < 0)
		{
			Color.G = 0;
		}
		if (Color.G > 255)
		{
			Color.G = 255;
		}
	}
	else
	{
		Color.G = Temperature_ - 60;
		Color.G = 288.1221695283 * (Color.G ^ -0.0755148492);
		
		if (Color.G < 0)
		{
			Color.G = 0;
		}
		if (Color.G > 255)
		{
			Color.G = 255;
		}
	}
	//----------GREEN----------
	
	//----------BLUE----------
	if (Temperature_ >= 66)
	{
		Color.B = 255;
	}
	else
	{
		if (Temperature_ <= 19)
		{
			Color.B = 0;
		}
		else
		{
			Color.B = Temperature_ - 10;
			Color.B = 138.5177312231 * Ln(Color.B) - 305.0447927307;

			if (Color.B < 0)
			{
				Color.B = 0;
			}
			if (Color.B > 255)
			{
				Color.B = 255;
			}
		}
	}
	//----------BLUE----------
}
*/
