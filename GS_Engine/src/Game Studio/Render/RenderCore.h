#pragma once

#include "Core.h"

enum class PixelFormat : uint8
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