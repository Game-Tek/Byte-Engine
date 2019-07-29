#pragma once

#include "Core.h"

//Specifies all available image layouts.
enum class ImageLayout : uint8
{
	GENERAL,
	COLOR_ATTACHMENT,
	DEPTH_STENCIL_ATTACHMENT,
	DEPTH_STENCIL_READ_ONLY,
	SHADER_READ,
	TRANSFER_SOURCE,
	TRANSFER_DESTINATION,
	PREINITIALIZED,
	PRESENTATION
};

//Specifies all available color formats and depth/stencil formats.
//Usually you'll use the DEPTH16_STENCIL8 since it is sufficient form most use cases. If that is not precise enough use the DEPTH24_STENCIL8.
enum class Format : uint8
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


	//  DEPTH STENCIL

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

//Describes all possible operations a renderer can perform when loading a render target onto a render pass.
enum class LoadOperations : uint8
{
	//We don't care about the previous content of the render target. Behavior is unknown.
	UNDEFINED,
	//We want to load the previous content of the render target.
	LOAD,
	//We want the render target to be cleared to black for color attachments and to 0 for depth/stencil attachments.
	CLEAR
};

//Describes all possible operations a renderer can perform when saving to a render target from a render pass.
enum class StoreOperations : uint8
{
	//We don't care about the outcome of the render target.
	UNDEFINED,
	//We want to store the result of the render pass to this render attachment.
	STORE
};

enum class ShaderType : uint8
{
	VERTEX_SHADER,
	FRAGMENT_SHADER,
	COMPUTE_SHADER
};

enum class ImageDimensions : uint8
{
	IMAGE_1D, IMAGE_2D, IMAGE_3D
};

enum class ImageType : uint8
{
	COLOR, DEPTH, STENCIL, DEPTH_STENCIL
};

enum class ImageUse : uint8
{
	TRANSFER_SOURCE,
	TRANSFER_DESTINATION,
	SAMPLE,
	STORAGE,
	COLOR_ATTACHMENT,
	DEPTH_STENCIL_ATTACHMENT,
	TRANSIENT_ATTACHMENT,
	INPUT_ATTACHMENT
};

enum class BufferType : uint8
{
	BUFFER_VERTEX,
	BUFFER_INDEX,
	BUFFER_UNIFORM
};

enum class ShaderDataTypes : uint8
{
	FLOAT,
	FLOAT2,
	FLOAT3,
	FLOAT4,

	INT,
	INT2,
	INT3,
	INT4,

	BOOL,

	MAT3,
	MAT4
};