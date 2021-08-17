#pragma once

#include "RenderCore.h"
#include <GTSL/Extent.h>
#include <GTSL/Range.h>
#include <shaderc/shaderc.h>
#include <shaderc/shaderc.hpp>

#include "GTSL/Buffer.hpp"
#include "GTSL/ShortString.hpp"

namespace GAL
{
	struct StencilOperations
	{
		struct StencilState
		{
			StencilCompareOperation FailOperation = StencilCompareOperation::ZERO;
			StencilCompareOperation PassOperation = StencilCompareOperation::ZERO;
			StencilCompareOperation DepthFailOperation = StencilCompareOperation::ZERO;
			CompareOperation CompareOperation = CompareOperation::NEVER;
			GTSL::uint32 CompareMask;
			GTSL::uint32 WriteMask;
			GTSL::uint32 Reference;
		} Front, Back;
	};

	class Shader
	{
	public:
	};
	
	class RenderPass;

	struct PushConstant
	{
		GTSL::uint32 NumberOf4ByteSlots = 0;
		ShaderStage Stage;
	};
	
	class Pipeline
	{
	public:

		static constexpr GTSL::uint8 MAX_VERTEX_ELEMENTS = 20;

		struct VertexElement {
			GTSL::ShortString<32> Identifier;
			ShaderDataType Type;
		};

		static constexpr auto POSITION = GTSL::ShortString<32>(u8"POSITION");
		static constexpr auto NORMAL = GTSL::ShortString<32>(u8"NORMAL");
		static constexpr auto TANGENT = GTSL::ShortString<32>(u8"TANGENT");
		static constexpr auto BITANGENT = GTSL::ShortString<32>(u8"BITANGENT");
		static constexpr auto TEXTURE_COORDINATES = GTSL::ShortString<32>(u8"TEXTURE_COORDINATES");
		static constexpr auto COLOR = GTSL::ShortString<32>(u8"COLOR");

		struct RayTraceGroup {
			static constexpr GTSL::uint32 SHADER_UNUSED = (~0U);
			
			ShaderGroupType ShaderGroup;
			GTSL::uint32 GeneralShader;
			GTSL::uint32 ClosestHitShader;
			GTSL::uint32 AnyHitShader;
			GTSL::uint32 IntersectionShader;
		};
		
		struct PipelineStateBlock
		{
			struct ViewportState {
				GTSL::uint8 ViewportCount = 0;
			};

			struct RasterState {				
				WindingOrder WindingOrder = WindingOrder::CLOCKWISE;
				CullMode CullMode = CullMode::CULL_BACK;
			};

			struct DepthState {
				CompareOperation CompareOperation = CompareOperation::LESS;
			};

			struct RenderContext {
				struct AttachmentState {
					FormatDescriptor FormatDescriptor;
					bool BlendEnable = true;
				};

				GTSL::Range<const AttachmentState*> Attachments;
				const RenderPass* RenderPass = nullptr;
				GTSL::uint8 SubPassIndex = 0;
			};

			struct VertexState {
				GTSL::Range<const VertexElement*> VertexDescriptor;
			};

			struct RayTracingState {
				GTSL::Range<const RayTraceGroup*> Groups;
				GTSL::uint8 MaxRecursionDepth;
			};

			union {
				ViewportState Viewport;
				RasterState Raster;
				DepthState Depth;
				RenderContext Context;
				VertexState Vertex;
				RayTracingState RayTracing;
			};			

			enum class StateType
			{
				VIEWPORT_STATE, RASTER_STATE, DEPTH_STATE, COLOR_BLEND_STATE, VERTEX_STATE, RAY_TRACE_GROUPS
			} Type;
			
			PipelineStateBlock() = default;
			PipelineStateBlock(const RasterState& rasterState) : Raster(rasterState), Type(StateType::RASTER_STATE) {}
			PipelineStateBlock(const DepthState& depth) : Depth(depth), Type(StateType::DEPTH_STATE) {}
			PipelineStateBlock(const RenderContext& renderContext) : Context(renderContext), Type(StateType::COLOR_BLEND_STATE) {}
			PipelineStateBlock(const VertexState& vertexState) : Vertex(vertexState), Type(StateType::VERTEX_STATE) {}
			PipelineStateBlock(const ViewportState& viewportState) : Viewport(viewportState), Type(StateType::VIEWPORT_STATE) {}
			PipelineStateBlock(const RayTracingState& rayTracingGroups) : RayTracing(rayTracingGroups), Type(StateType::RAY_TRACE_GROUPS) {}
		};
		
		//struct ShaderInfo
		//{
		//	ShaderType Type = ShaderType::VERTEX_SHADER;
		//	const Shader* Shader = nullptr;
		//};
	};

	class PipelineCache
	{
	public:
	private:
	};
	
	class GraphicsPipeline : public Pipeline
	{
	public:
		
		GraphicsPipeline() = default;
		
		static GTSL::uint32 GetVertexSize(GTSL::Range<const ShaderDataType*> vertex)
		{
			GTSL::uint32 size{ 0 };	for (const auto& e : vertex) { size += ShaderDataTypesSize(e); } return size;
		}

		static GTSL::uint32 GetByteOffsetToMember(const GTSL::uint8 member, GTSL::Range<const ShaderDataType*> vertex)
		{
			GTSL::uint32 offset{ 0 };
			for (GTSL::uint8 i = 0; i < member; ++i) { offset += ShaderDataTypesSize(vertex[i]); }
			return offset;
		}
	};

	class ComputePipeline : public Pipeline
	{
	public:
	};

	template<class BUF, class STR>
	bool CompileShader(GTSL::Range<const char8_t*> code, GTSL::Range<const char8_t*> shaderName, ShaderType shaderType, ShaderLanguage shaderLanguage, BUF& result, STR& stringResult)
	{
		shaderc_shader_kind shaderc_stage;

		switch (shaderType)
		{
		case ShaderType::VERTEX: shaderc_stage = shaderc_vertex_shader;	break;
		case ShaderType::TESSELLATION_CONTROL: shaderc_stage = shaderc_tess_control_shader;	break;
		case ShaderType::TESSELLATION_EVALUATION: shaderc_stage = shaderc_tess_evaluation_shader; break;
		case ShaderType::GEOMETRY: shaderc_stage = shaderc_geometry_shader;	break;
		case ShaderType::FRAGMENT: shaderc_stage = shaderc_fragment_shader;	break;
		case ShaderType::COMPUTE: shaderc_stage = shaderc_compute_shader; break;
		case ShaderType::RAY_GEN: shaderc_stage = shaderc_raygen_shader; break;
		case ShaderType::CLOSEST_HIT: shaderc_stage = shaderc_closesthit_shader; break;
		case ShaderType::ANY_HIT: shaderc_stage = shaderc_anyhit_shader; break;
		case ShaderType::INTERSECTION: shaderc_stage = shaderc_intersection_shader; break;
		case ShaderType::MISS: shaderc_stage = shaderc_miss_shader; break;
		case ShaderType::CALLABLE: shaderc_stage = shaderc_callable_shader; break;
		default: GAL_DEBUG_BREAK;
		}

		const shaderc::Compiler shaderc_compiler;
		shaderc::CompileOptions shaderc_compile_options;
		shaderc_compile_options.SetTargetSpirv(shaderc_spirv_version_1_5);
		shaderc_compile_options.SetTargetEnvironment(shaderc_target_env_vulkan, shaderc_env_version_vulkan_1_2);

		shaderc_source_language shaderc_source_language;
		switch (shaderLanguage)
		{
		case ShaderLanguage::GLSL: shaderc_source_language = shaderc_source_language_glsl; break;
		case ShaderLanguage::HLSL: shaderc_source_language = shaderc_source_language_hlsl; break;
		default: GAL_DEBUG_BREAK;
		}

		shaderc_compile_options.SetSourceLanguage(shaderc_source_language);
		shaderc_compile_options.SetOptimizationLevel(shaderc_optimization_level_performance);
		const auto shaderc_module = shaderc_compiler.CompileGlslToSpv(reinterpret_cast<const char*>(code.begin()), code.Bytes(), shaderc_stage, reinterpret_cast<const char*>(shaderName.begin()), shaderc_compile_options);

		if (shaderc_module.GetCompilationStatus() != shaderc_compilation_status_success) {
			auto errorString = shaderc_module.GetErrorMessage();
			stringResult += GTSL::Range<const char8_t*>(errorString.size() + 1, reinterpret_cast<const char8_t*>(errorString.c_str()));
			return false;
		}

		result.CopyBytes((shaderc_module.end() - shaderc_module.begin()) * sizeof(GTSL::uint32), reinterpret_cast<const GTSL::byte*>(shaderc_module.begin()));

		return true;
	}
}
