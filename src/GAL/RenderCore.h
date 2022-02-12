#pragma once

#include <GTSL/Core.h>

#include <GTSL/Flags.h>

#include <GTSL/Algorithm.hpp>

#include "GTSL/Bitman.h"
#include "GTSL/Extent.h"
#include "GTSL/RGB.h"
#include "GTSL/Math/Matrix4.h"

#undef OPAQUE

namespace GAL {
	class Texture;
	class TextureView;

	template<typename T>
	constexpr void debugClear(T& handle) { if constexpr (BE_DEBUG) { handle = reinterpret_cast<T>(0); } }
	
	constexpr GTSL::uint8 MAX_SHADER_STAGES = 8;

	template<typename FVR, class FVT, typename Y, typename Z>
	void TranslateMask(const GTSL::Flags<FVR, FVT> fromValue, const Y toValue, const GTSL::Flags<FVR, FVT> fromVar, Z& toVar) {
		GTSL::SetBitAs(GTSL::FFSB(toValue), static_cast<FVR>(fromVar) & static_cast<FVR>(fromValue), toVar);
	}

	template<typename FVR, class FVT, typename Y, typename Z>
	void TranslateMask(const Y fromValue, Z fromVar, const GTSL::Flags<FVR, FVT> toValue, GTSL::Flags<FVR, FVT>& toVariable) {
		GTSL::SetBitAs(GTSL::FFSB(toValue), fromVar & fromValue, static_cast<FVR&>(toVariable));
	}
	
	enum class RenderAPI : GTSL::uint8 {
		VULKAN,
		DIRECTX12
	};

	using MemoryType = GTSL::Flags<GTSL::uint8, struct MemoryTypeTag>;

	namespace MemoryTypes {
		static constexpr MemoryType GPU(1), HOST_VISIBLE(2), HOST_COHERENT(4), HOST_CACHED(8);
	}
	
	struct MemoryRequirements {
		GTSL::uint32 Size{ 0 };
		GTSL::uint32 Alignment{ 0 }, MemoryTypes{ 0 };
		//MemoryType MemoryTypes;
	};
	
	using PipelineStage = GTSL::Flags<GTSL::uint32, struct PipelineStageTag>;

	inline GTSL::uint16 FloatToUNORM(const GTSL::float32 x) {
		return static_cast<GTSL::uint16>(x * 65535);
	}

	inline GTSL::int16 FloatToSNORM(const GTSL::float32 x) {
		//According to D3D10 rules, the value "-1.0f" has two representations:
		//  0x1000 and 0x10001
		//This allows everyone to convert by just multiplying by 32767 instead
		//of multiplying the negative values by 32768 and 32767 for positive.
		return static_cast<int16>(GTSL::Math::Clamp(x >= 0.0f ? (x * 32767.0f + 0.5f) : (x * 32767.0f - 0.5f), -32768.0f, 32767.0f));
	}
	
	namespace PipelineStages {
		static constexpr PipelineStage TOP_OF_PIPE(1),
			DRAW_INDIRECT(2),
			VERTEX_INPUT(4),
			VERTEX(8),
			TESSELLATION_CONTROL(16),
			TESSELLATION_EVALUATION(32),
			GEOMETRY(64),
			FRAGMENT(128),
			EARLY_FRAGMENT_TESTS(256),
			LATE_FRAGMENT_TESTS(512),
			COLOR_ATTACHMENT_OUTPUT(1024),
			COMPUTE(2048),
			TRANSFER(4096),
			BOTTOM_OF_PIPE(8192),
			HOST(16384),
			ALL_GRAPHICS(32768),
			RAY_TRACING(0x00200000),
			ACCELERATION_STRUCTURE_BUILD(0x02000000),
			SHADING_RATE_IMAGE(0x00400000),
			TASK(0x00080000),
			MESH(0x00100000);
	}

	constexpr GTSL::uint8 RAY_GEN_TABLE_INDEX = 0, HIT_TABLE_INDEX = 1, MISS_TABLE_INDEX = 2, CALLABLE_TABLE_INDEX = 3;

	enum class ComponentType : GTSL::uint8 { INT, UINT, FLOAT, NON_LINEAR };
	enum class TextureType : GTSL::uint8 { COLOR, DEPTH };

	struct DeviceAddress {
		DeviceAddress() = default;
		explicit DeviceAddress(const GTSL::uint64 add) : address(add) {}
		
		explicit operator GTSL::uint64() const { return address; }
		explicit operator bool() const { return address; }
		DeviceAddress operator+(const GTSL::uint64 add) const { return DeviceAddress(address + add); }
		DeviceAddress& operator+=(uint32 offset) { address += offset; return *this; }
	private:
		GTSL::uint64 address = 0;
	};
	
	struct ShaderHandle {
		ShaderHandle() = default;
		GTSL::byte size[32];
	};

	inline GTSL::uint32 SizeFromExtent(const GTSL::Extent3D extent) {
		return extent.Width * extent.Height * extent.Depth;
	}

	constexpr GTSL::uint32 bitExtracted(GTSL::uint32 number, GTSL::uint8 k, GTSL::uint8 p) {
		return (((1 << k) - 1) & (number >> (p - 1)));
	}

	enum class ColorSpaces : GTSL::uint8 {
		LINEAR, SRGB_NONLINEAR, DISPLAY_P3_LINEAR, DISPLAY_P3_NONLINEAR, HDR10_ST2048, DOLBY_VISION, HDR10_HLG, ADOBERGB_LINEAR, ADOBERGB_NONLINEAR
	};

	struct FormatDescriptor
	{
		FormatDescriptor() = default;
		
		constexpr FormatDescriptor(ComponentType compType, GTSL::uint8 compCount, GTSL::uint8 bitDepth, TextureType type, GTSL::uint8 a, GTSL::uint8 b, GTSL::uint8 c, GTSL::uint8 d, ColorSpaces color_space = ColorSpaces::LINEAR) :
		Component(compType), ComponentCount(compCount), A(a), B(b), C(c), D(d), BitDepth(GTSL::FindFirstSetBit(bitDepth).Get()), Type(type), ColorSpace(color_space)
		{}

		constexpr FormatDescriptor(const GTSL::uint32 i) : Component(static_cast<ComponentType>(bitExtracted(i, 4, 0))),
		ComponentCount(bitExtracted(i, 4, 4)), A(bitExtracted(i, 2, 8)), B(bitExtracted(i, 2, 10)), C(bitExtracted(i, 2, 12)), D(bitExtracted(i, 2, 14)),
		BitDepth(bitExtracted(i, 3, 16)), Type(static_cast<TextureType>(bitExtracted(i, 2, 19))), ColorSpace(static_cast<ColorSpaces>(bitExtracted(i, 4, 21))) {}
		
		ComponentType Component : 4; //0
		GTSL::uint8 ComponentCount : 4;  //4
		GTSL::uint8 A : 2;               //8
		GTSL::uint8 B : 2;               //10
		GTSL::uint8 C : 2;               //12
		GTSL::uint8 D : 2;               //14
		GTSL::uint8 BitDepth : 3;        //16
		TextureType Type : 2;            //19
		ColorSpaces ColorSpace : 4;      //21

		[[nodiscard]] GTSL::uint8 GetBitDepth() const { return static_cast<GTSL::uint8>(1) << BitDepth; }
		[[nodiscard]] GTSL::uint8 GetSize() const { return GetBitDepth() / 8 * ComponentCount; }

		[[nodiscard]] constexpr operator GTSL::uint32() const {
			return static_cast<GTSL::UnderlyingType<ComponentType>>(Component) | ComponentCount << 4 | A << 8 | B << 10 | C << 12 | D << 14 | BitDepth << 16 | static_cast<GTSL::UnderlyingType<TextureType>>(Type) << 19 | static_cast<GTSL::UnderlyingType<ColorSpaces>>(ColorSpace) << 21;
		}
	};
	
	namespace FORMATS {
		static constexpr auto R_I8 = FormatDescriptor(ComponentType::INT, 1, 8, TextureType::COLOR, 0, 0, 0, 0);
		static constexpr auto R_SRGB_I8 = FormatDescriptor(ComponentType::INT, 1, 8, TextureType::COLOR, 0, 0, 0, 0, ColorSpaces::SRGB_NONLINEAR);
		static constexpr auto RGB_I8 = FormatDescriptor(ComponentType::INT, 3, 8, TextureType::COLOR, 0, 1, 2, 3);
		static constexpr auto BGRA_I8 = FormatDescriptor(ComponentType::INT, 4, 8, TextureType::COLOR, 2, 1, 0, 3);
		static constexpr auto RG_I32 = FormatDescriptor(ComponentType::INT, 2, 32, TextureType::COLOR, 0, 1, 0, 0);
		static constexpr auto BGRA_NONLINEAR8 = FormatDescriptor(ComponentType::NON_LINEAR, 4, 8, TextureType::COLOR, 2, 1, 0, 3);
		static constexpr auto RGBA_F16 = FormatDescriptor(ComponentType::FLOAT, 4, 16, TextureType::COLOR, 0, 1, 2, 3);
		static constexpr auto RGBA_I8 = FormatDescriptor(ComponentType::INT, 4, 8, TextureType::COLOR, 0, 1, 2, 3);
		static constexpr auto RGBA_SRGB_I8 = FormatDescriptor(ComponentType::INT, 4, 8, TextureType::COLOR, 0, 1, 2, 3, ColorSpaces::SRGB_NONLINEAR);
		static constexpr auto DEPTH_F32 = FormatDescriptor(ComponentType::FLOAT, 1, 32, TextureType::DEPTH, 0, 0, 0, 0);
	}
	
	enum class Format {
		R_I8 = FORMATS::R_I8,
		R_SRGB_I8 = FORMATS::R_SRGB_I8,
		RGB_I8 = FORMATS::RGB_I8,
		RGBA_I8 = FORMATS::RGBA_I8,
		RGBA_SRGB_I8 = FORMATS::RGBA_SRGB_I8,
		RGBA_F16 = FORMATS::RGBA_F16,
		BGRA_I8 = FORMATS::BGRA_I8,
		RG_I32 = FORMATS::RG_I32,
		DEPTH32 = FORMATS::DEPTH_F32
	};

	constexpr Format MakeFormatFromFormatDescriptor(const FormatDescriptor formatDescriptor) {
		return static_cast<Format>(static_cast<GTSL::uint32>(formatDescriptor));
	}

	//constexpr FormatDescriptor MakeFormatDescriptorFromFormat(const Format format) {
	//	return FormatDescriptor(static_cast<GTSL::uint32>(format));
	//}
	
	class RenderDevice;
	
	using BindingFlag = GTSL::Flags<GTSL::uint8, struct BindingFlagTag>;
	namespace BindingFlags {
		static constexpr BindingFlag PARTIALLY_BOUND(1 << 0);
	}
	
	using ShaderStage = GTSL::Flags<GTSL::uint16, struct ShaderStageTag>;
	namespace ShaderStages {
		static constexpr ShaderStage VERTEX(1),
			TESSELLATION_CONTROL(2),
			TESSELLATION_EVALUATION(4),
			GEOMETRY(8),
			FRAGMENT(16),
			COMPUTE(32),
			TASK(64),
			MESH(128),
			RAY_GEN(256), ANY_HIT(512), CLOSEST_HIT(1024), MISS(2048), INTERSECTION(4096), CALLABLE(8192);
	};

	using TextureUse = GTSL::Flags<GTSL::uint32, struct TextureUseTag>;
	namespace TextureUses {
		static constexpr TextureUse TRANSFER_SOURCE(1), TRANSFER_DESTINATION(2), SAMPLE(4), STORAGE(8), ATTACHMENT(16), TRANSIENT_ATTACHMENT(32), INPUT_ATTACHMENT(64);
	}
	
	using QueueType = GTSL::Flags<GTSL::uint8, struct QueueTypeTag>;
	namespace QueueTypes {
		static constexpr QueueType GRAPHICS(1 << 0), COMPUTE(1 << 1), TRANSFER(1 << 2);
	}

	using BufferUse = GTSL::Flags< GTSL::uint32, struct BufferUseFlag>;
	namespace BufferUses {
		static constexpr BufferUse TRANSFER_SOURCE(1 << 0), TRANSFER_DESTINATION(1 << 1), STORAGE(1 << 2), ACCELERATION_STRUCTURE(1 << 3), ADDRESS(1 << 4), UNIFORM(1 << 5), VERTEX(1 << 6), INDEX(1 << 7), SHADER_BINDING_TABLE(1 << 8), BUILD_INPUT_READ(1 << 9);
	};

	using AllocationFlag = GTSL::Flags<GTSL::uint8, struct AllocationFlagTag>;
	namespace AllocationFlags {
		static constexpr AllocationFlag DEVICE_ADDRESS(1), DEVICE_ADDRESS_CAPTURE_REPLAY(2);
	}

	using AccessType = GTSL::Flags<GTSL::uint8, struct AccessTypeFlag>;
	namespace AccessTypes {
		static constexpr AccessType READ(1), WRITE(4);
	}
	
	using AccessFlag = GTSL::Flags<GTSL::uint32, struct AccessFlagTag>;	
	namespace AccessFlags {
		static constexpr AccessFlag INDIRECT_COMMAND_READ(1 << 0),
		INDEX_READ(1 << 1),
		VERTEX_ATTRIBUTE_READ(1 << 2),
		UNIFORM_READ(1 << 3),
		INPUT_ATTACHMENT_READ(1 << 4),
		SHADER_READ(1 << 5),
		SHADER_WRITE(1 << 6),
		ATTACHMENT_READ(1 << 7),
		ATTACHMENT_WRITE(1 << 8),
		TRANSFER_READ(1 << 11),
		TRANSFER_WRITE(1 << 12),
		HOST_READ(1 << 13),
		HOST_WRITE(1 << 14),
		MEMORY_READ(1 << 15),
		MEMORY_WRITE(1 << 16),
		ACCELERATION_STRUCTURE_READ(1 << 17),
		ACCELERATION_STRUCTURE_WRITE(1 << 18),
		SHADING_RATE_IMAGE_READ(1 << 19);
	}
	
	// IMAGE

	//Specifies all available image layouts.
	enum class TextureLayout : GTSL::uint8 {
		UNDEFINED,
		GENERAL,
		ATTACHMENT,
		SHADER_READ,
		TRANSFER_SOURCE,
		TRANSFER_DESTINATION,
		PREINITIALIZED,
		PRESENTATION
	};

	enum class GeometryType {
		TRIANGLES, AABB, INSTANCES
	};

	enum class QueryType {
		COMPACT_ACCELERATION_STRUCTURE_SIZE
	};
	
	using GeometryFlag = GTSL::Flags<GTSL::uint8, struct GeometryFlagTag>;
	namespace GeometryFlags {
		static constexpr GeometryFlag OPAQUE(1 << 0);
	}
	
	using AccelerationStructureFlag = GTSL::Flags<GTSL::uint8, struct AccelerationStructureFlagTag>;
	namespace AccelerationStructureFlags {
		static constexpr AccelerationStructureFlag ALLOW_UPDATE(1 << 0), ALLOW_COMPACTION(1 << 1), PREFER_FAST_TRACE(1 << 2), PREFER_FAST_BUILD(1 << 3), LOW_MEMORY(1 << 4);
	}
	
	enum class Tiling {
		OPTIMAL, LINEAR
	};

	enum class Device : GTSL::uint8 {
		GPU, CPU, GPU_OR_CPU
	};
	
	// ATTACHMENTS

	//Describes all possible operations a GAL can perform when loading a render target onto a render pass.
	enum class Operations : GTSL::uint8 {
		//We don't care about the previous content of the render target. Behavior is unknown.
		UNDEFINED,
		//We want to load the previous content of the render target.
		DO,
		//We want the render target to be cleared to black for color attachments and to 0 for depth/stencil attachments.
		CLEAR
	};

	enum class SampleCount : GTSL::uint8 {
		SAMPLE_COUNT_1,
		SAMPLE_COUNT_2,
		SAMPLE_COUNT_4,
		SAMPLE_COUNT_8,
		SAMPLE_COUNT_16,
		SAMPLE_COUNT_32,
		SAMPLE_COUNT_64
	};

	// SHADERS

	enum class ShaderLanguage : GTSL::uint8 {
		GLSL, HLSL
	};
	
	enum class ShaderType : GTSL::uint8 {
		VERTEX,
		TESSELLATION_CONTROL,
		TESSELLATION_EVALUATION,
		GEOMETRY,
		FRAGMENT,

		COMPUTE,

		TASK, MESH,

		RAY_GEN, CLOSEST_HIT, ANY_HIT, INTERSECTION, MISS, CALLABLE
	};

	enum class ShaderDataType : GTSL::uint8 {
		FLOAT,
		FLOAT2,
		FLOAT3,
		FLOAT4,

		UINT16,
		UINT32,
		UINT64,

		INT,
		INT2,
		INT3,
		INT4,

		BOOL,

		MAT3,
		MAT4,

		U16_SNORM, U16_SNORM2, U16_SNORM3, U16_SNORM4,
		U16_UNORM, U16_UNORM2, U16_UNORM3, U16_UNORM4
	};

	// PIPELINE

	enum class CullMode : GTSL::uint8 {
		CULL_NONE,
		CULL_FRONT,
		CULL_BACK
	};

	enum class WindingOrder : GTSL::uint8 {
		CLOCKWISE,
		COUNTER_CLOCKWISE
	};

	enum class BlendOperation : GTSL::uint8 {
		WRITE,
		ADD,
		SUBTRACT,
		REVERSE_SUBTRACT,
		MIN,
		MAX
	};

	enum class CompareOperation : GTSL::uint8 {
		NEVER,
		LESS,
		EQUAL,
		LESS_OR_EQUAL,
		GREATER,
		NOT_EQUAL,
		GREATER_OR_EQUAL,
		ALWAYS
	};

	enum class StencilCompareOperation : GTSL::uint8 {
		KEEP,
		ZERO,
		REPLACE,
		INCREMENT_AND_CLAMP,
		DECREMENT_AND_CLAMP,
		INVERT,
		INCREMENT_AND_WRAP,
		DECREMENT_AND_WRAP
	};

	enum class BindingType : GTSL::uint8 {
		SAMPLER = 0,
		COMBINED_IMAGE_SAMPLER = 1,
		SAMPLED_IMAGE = 2,
		STORAGE_IMAGE = 3,
		UNIFORM_TEXEL_BUFFER = 4,
		STORAGE_TEXEL_BUFFER = 5,
		UNIFORM_BUFFER = 6,
		STORAGE_BUFFER = 7,
		UNIFORM_BUFFER_DYNAMIC = 8,
		STORAGE_BUFFER_DYNAMIC = 9,
		INPUT_ATTACHMENT = 10,
		ACCELERATION_STRUCTURE = 11
	};

	enum class PresentModes : GTSL::uint8 {
		/**
		* \brief All rendered images are queued in FIFO fashion and presented at V-BLANK. Best for when latency is not that important and energy consumption is.
		*/
		FIFO = 0,
		
		/**
		* \brief The last rendered image is the one which will be presented. Best for when latency is important and energy consumption is not.
		*/
		SWAP = 1
	};

	enum class ShaderGroupType {
		GENERAL, TRIANGLES, PROCEDURAL
	};

	enum class IndexType { UINT8 = 1, UINT16 = 2, UINT32 = 4 };

	inline uint32 IndexSize(const IndexType indexType) {
		return static_cast<uint32>(indexType);
	}

	struct RenderPassTargetDescription {
		//AccessType AccessType;
		Operations LoadOperation, StoreOperation;
		TextureLayout Start, End;
		FormatDescriptor FormatDescriptor;
		const Texture* Texture = nullptr;
		const TextureView* TextureView = nullptr;
		GTSL::RGBA ClearValue;
	};

	struct RayTracingInstance {
		GTSL::Matrix3x4 Transform;
		GTSL::uint32 InstanceIndex : 24;
		GTSL::uint32 Mask : 8;
		GTSL::uint32 InstanceShaderBindingTableRecordOffset : 24;
		GeometryFlag Flags; //8 bits
		DeviceAddress AccelerationStructureAddress;
	};
	
	inline GTSL::uint8 ShaderDataTypesSize(const ShaderDataType type) {
		switch (type) {
			case ShaderDataType::FLOAT: return 4;
			case ShaderDataType::FLOAT2: return 8;
			case ShaderDataType::FLOAT3: return 12;
			case ShaderDataType::FLOAT4: return 16;
			case ShaderDataType::INT: return 4;
			case ShaderDataType::INT2: return 8;
			case ShaderDataType::INT3: return 12;
			case ShaderDataType::INT4: return 16;
			case ShaderDataType::BOOL: return 1;
			case ShaderDataType::MAT3: return 36;
			case ShaderDataType::MAT4: return 64;
			default: __debugbreak();
		}
		
		return 0;
	}

	inline IndexType SizeToIndexType(const GTSL::uint8 size) {
		switch (size) {
		case 1: return IndexType::UINT8;
		case 2: return IndexType::UINT16;
		case 4: return IndexType::UINT32;
		}
	}

	inline ShaderStage ShaderTypeToShaderStageFlag(ShaderType type) {
		switch (type) {
		case ShaderType::VERTEX: return ShaderStages::VERTEX;
		case ShaderType::FRAGMENT: return ShaderStages::FRAGMENT;
		case ShaderType::COMPUTE: return ShaderStages::COMPUTE;
		case ShaderType::TASK: return ShaderStages::TASK;
		case ShaderType::MESH: return ShaderStages::MESH;
		case ShaderType::RAY_GEN: return ShaderStages::RAY_GEN;
		case ShaderType::ANY_HIT: return ShaderStages::ANY_HIT;
		case ShaderType::CLOSEST_HIT: return ShaderStages::CLOSEST_HIT;
		case ShaderType::MISS: return ShaderStages::MISS;
		case ShaderType::INTERSECTION: return ShaderStages::INTERSECTION;
		case ShaderType::CALLABLE: return ShaderStages::CALLABLE;
		}

		return ShaderStage();
	}

#if (_WIN32)
#define GAL_DEBUG_BREAK __debugbreak();
#endif
}
