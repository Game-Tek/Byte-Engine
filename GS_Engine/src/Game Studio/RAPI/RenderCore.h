#pragma once

#include "Core.h"

namespace RAPI
{

	constexpr uint8 MAX_SHADER_STAGES = 8;

	struct RenderInfo
	{
		class RenderDevice* RenderDevice = nullptr;
	};

	// IMAGE

	//Specifies all available image layouts.
	enum class ImageLayout : uint8
	{
		UNDEFINED,
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

	enum class ImageDimensions : uint8
	{
		IMAGE_1D,
		IMAGE_2D,
		IMAGE_3D
	};

	enum class ImageType : uint8
	{
		COLOR,
		DEPTH,
		STENCIL,
		DEPTH_STENCIL
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

	//Specifies all available color formats and depth/stencil formats.
	//Usually you'll use the DEPTH16_STENCIL8 since it is sufficient form most use cases. If that is not precise enough use the DEPTH24_STENCIL8.
	enum class ImageFormat : uint8
	{
		//INTEGER

		//R
		R_I8,
		R_I16,
		R_I32,
		R_I64,
		//RG
		RG_I8,
		RG_I16,
		RG_I32,
		RG_I64,
		//RBG
		RGB_I8,
		RGB_I16,
		RGB_I32,
		RGB_I64,
		//RGBA
		RGBA_I8,
		RGBA_I16,
		RGBA_I32,
		RGBA_I64,
		//RGBA
		BGRA_I8,

		BGR_I8,

		//FLOATING POINT

		//R
		R_F16,
		R_F32,
		R_F64,
		//RG
		RG_F16,
		RG_F32,
		RG_F64,
		//RBG
		RGB_F16,
		RGB_F32,
		RGB_F64,
		//RGBA
		RGBA_F16,
		RGBA_F32,
		RGBA_F64,


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


	// ATTACHMENTS

	//Describes all possible operations a RAPI can perform when loading a render target onto a render pass.
	enum class RenderTargetLoadOperations : uint8
	{
		//We don't care about the previous content of the render target. Behavior is unknown.
		UNDEFINED,
		//We want to load the previous content of the render target.
		LOAD,
		//We want the render target to be cleared to black for color attachments and to 0 for depth/stencil attachments.
		CLEAR
	};

	//Describes all possible operations a RAPI can perform when saving to a render target from a render pass.
	enum class RenderTargetStoreOperations : uint8
	{
		//We don't care about the outcome of the render target.
		UNDEFINED,
		//We want to store the result of the render pass to this render attachment.
		STORE
	};

	enum class SampleCount : uint8
	{
		SAMPLE_COUNT_1,
		SAMPLE_COUNT_2,
		SAMPLE_COUNT_4,
		SAMPLE_COUNT_8,
		SAMPLE_COUNT_16,
		SAMPLE_COUNT_32,
		SAMPLE_COUNT_64
	};

	// SHADERS

	enum class ShaderType : uint8
	{
		ALL_STAGES,

		VERTEX_SHADER,
		TESSELLATION_CONTROL_SHADER,
		TESSELLATION_EVALUATION_SHADER,
		GEOMETRY_SHADER,
		FRAGMENT_SHADER,

		COMPUTE_SHADER
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
		MAT4,

		TEXTURE_1D,
		TEXTURE_2D,
		TEXTURE_3D,

		TEXTURE_2D_CUBE,
	};


	// BUFFERS

	enum class BufferType : uint8
	{
		BUFFER_VERTEX,
		BUFFER_INDEX,
		BUFFER_UNIFORM
	};


	// PIPELINE

	enum class CullMode : uint8
	{
		CULL_NONE,
		CULL_FRONT,
		CULL_BACK
	};

	enum class BlendOperation : uint8
	{
		ADD,
		SUBTRACT,
		REVERSE_SUBTRACT,
		MIN,
		MAX
	};

	enum class CompareOperation : uint8
	{
		NEVER,
		LESS,
		EQUAL,
		LESS_OR_EQUAL,
		GREATER,
		NOT_EQUAL,
		GREATER_OR_EQUAL,
		ALWAYS
	};

	enum class StencilCompareOperation : uint8
	{
		KEEP,
		ZERO,
		REPLACE,
		INCREMENT_AND_CLAMP,
		DECREMENT_AND_CLAMP,
		INVERT,
		INCREMENT_AND_WRAP,
		DECREMENT_AND_WRAP
	};

	enum class BindingType : uint8
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
		MAT4,

		TEXTURE_1D,
		TEXTURE_2D,
		TEXTURE_3D,

		TEXTURE_2D_CUBE,
		
		SAMPLER,
		COMBINED_IMAGE_SAMPLER,
		SAMPLED_IMAGE,
		STORAGE_IMAGE,
		UNIFORM_TEXEL_BUFFER,
		STORAGE_TEXEL_BUFFER,
		UNIFORM_BUFFER,
		STORAGE_BUFFER,
		UNIFORM_BUFFER_DYNAMIC,
		STORAGE_BUFFER_DYNAMIC,
		INPUT_ATTACHMENT
	};

	/**
	 * \brief Enumeration of all possible presentation modes, which define the order at which the rendered images are presented to the screen.
	 */
	enum class PresentMode : uint8
	{
		/**
		 * \brief All rendered images are queued in FIFO fashion and presented at V-BLANK. Best for when latency is not that important and energy consumption is.
		 */
		FIFO,
		/**
		 * \brief The last rendered image is the one which will be presented. Best for when latency is important and energy consumption is not.
		 */
		SWAP,
	};

	INLINE uint8 ShaderDataTypesSize(ShaderDataTypes _SDT)
	{
		switch (_SDT)
		{
		case ShaderDataTypes::FLOAT: return 4;
		case ShaderDataTypes::FLOAT2: return 4 * 2;
		case ShaderDataTypes::FLOAT3: return 4 * 3;
		case ShaderDataTypes::FLOAT4: return 4 * 4;
		case ShaderDataTypes::INT: return 4;
		case ShaderDataTypes::INT2: return 4 * 2;
		case ShaderDataTypes::INT3: return 4 * 3;
		case ShaderDataTypes::INT4: return 4 * 4;
		case ShaderDataTypes::BOOL: return 4;
		case ShaderDataTypes::MAT3: return 4 * 3 * 3;
		case ShaderDataTypes::MAT4: return 4 * 4 * 4;
		default: return 0;
		}
	}

	class RAPIObject
	{
	public:
		virtual void Destroy(class RenderDevice* renderDevice);
	};

}