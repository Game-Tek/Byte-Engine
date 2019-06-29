#pragma once

#include "Core.h"

enum class ColorFormat : uint8
{
	//INTEGER

	//R
	R_I8, R_I16, R_I32, R_I64,
	//RG
	RG_I8, RG_I16, RG_I32, RG_I64,
	//RBG
	RGB_I8, RGB_I16, RGB_I32, RGB_I64,
	//RGBA
	RGBA_I8, RGBA_I16, RGBA_I32, RGBA_I64,
	//RGBA
	BGRA_I8,

	//FLOATING POINT

	//R
	R_F16, R_F32, R_F64,
	//RG
	RG_F16, RG_F32, RG_F64,
	//RBG
	RGB_F16, RGB_F32, RGB_F64,
	//RGBA
	RGBA_F16, RGBA_F32, RGBA_F64,
};

//Specifies all available depth/stencil formats.
//Usually you'll use the DEPTH16_STENCIL8 since it is sufficient form most use cases. If that is not precise enough use the DEPTH24_STENCIL8.
enum class DepthStencilFormat : uint8
{
	//A depth-only format with a 16 bit (2 byte) size.
	DEPTH16,
	//A depth-only format with a 32 (4 byte) bit size.
	DEPTH32,
	//A depth/stencil format with a 16 bit (2 byte) size depth part and an 8 bit (1 byte) size stencil part.
	DEPTH16_STENCIL8,
	//A depth/stencil format with a 24 bit (3 byte) size depth part and an 8 bit (1 byte) size stencil part.
	DEPTH24_STENCIL8,
	//A depth/stencil format with a 32 bit (4 byte) size depth part and an 8 bit (1 byte) size stencil part.
	DEPTH32_STENCIL8
};

//Specifies all available color spaces.
enum class ColorSpace : uint8
{
	//The non linear SRGB color space is the most commonly used color space to display things on screen. Use this when you are not developing an HDR application.
	NONLINEAR_SRGB,
	//The HDR10 represents a 10 bit color space which allows for more color information / depth. Use this when you are developing an HDR application.
	HDR10
};