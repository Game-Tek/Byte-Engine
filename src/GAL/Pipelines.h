#pragma once

#include "RenderCore.h"
#include <GTSL/Extent.h>
#include <GTSL/Range.hpp>
#include <shaderc/shaderc.h>
#include <shaderc/shaderc.hpp>

#include "GTSL/Buffer.hpp"
#include "GTSL/ShortString.hpp"

#include <dxgi.h>
#include <dxc/dxcapi.h>

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

	struct PushConstant {
		GTSL::uint32 NumberOf4ByteSlots = 0;
		ShaderStage Stage;
	};
	
	class Pipeline {
	public:

		static constexpr GTSL::uint8 MAX_VERTEX_ELEMENTS = 20;

		struct VertexElement {
			GTSL::ShortString<32> Identifier;
			ShaderDataType Type;
			uint8 Location = 0xFF;
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
			GTSL::uint32 GeneralShader = SHADER_UNUSED;
			GTSL::uint32 ClosestHitShader = SHADER_UNUSED;
			GTSL::uint32 AnyHitShader = SHADER_UNUSED;
			GTSL::uint32 IntersectionShader = SHADER_UNUSED;
		};
		
		struct PipelineStateBlock {
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
				//const RenderPass* RenderPass = nullptr;
				//GTSL::uint8 SubPassIndex = 0;
			};

			struct VertexState {
				GTSL::Range<const GTSL::Range<const VertexElement*>*> VertexStreams;
			};

			struct RayTracingState {
				GTSL::Range<const RayTraceGroup*> Groups;
				GTSL::uint8 MaxRecursionDepth;
			};

			struct SpecializationData {
				struct SpecializationEntry {
					uint64 Size, Offset, ID;
				};

				GTSL::Range<const SpecializationEntry*> Entries;
				GTSL::Range<const byte*> Data;
			};

			union {
				ViewportState Viewport;
				RasterState Raster;
				DepthState Depth;
				RenderContext Context;
				VertexState Vertex;
				RayTracingState RayTracing;
				SpecializationData Specialization;
			};			

			enum class StateType {
				VIEWPORT_STATE, RASTER_STATE, DEPTH_STATE, COLOR_BLEND_STATE, VERTEX_STATE, RAY_TRACE_GROUPS, SPECIALIZATION
			} Type;
			
			PipelineStateBlock() = default;
			PipelineStateBlock(const RasterState& rasterState) : Raster(rasterState), Type(StateType::RASTER_STATE) {}
			PipelineStateBlock(const DepthState& depth) : Depth(depth), Type(StateType::DEPTH_STATE) {}
			PipelineStateBlock(const RenderContext& renderContext) : Context(renderContext), Type(StateType::COLOR_BLEND_STATE) {}
			PipelineStateBlock(const VertexState& vertexState) : Vertex(vertexState), Type(StateType::VERTEX_STATE) {}
			PipelineStateBlock(const ViewportState& viewportState) : Viewport(viewportState), Type(StateType::VIEWPORT_STATE) {}
			PipelineStateBlock(const RayTracingState& rayTracingGroups) : RayTracing(rayTracingGroups), Type(StateType::RAY_TRACE_GROUPS) {}
			PipelineStateBlock(const SpecializationData& specialization_data) : Specialization(specialization_data), Type(StateType::SPECIALIZATION) {}
		};
		
		//struct ShaderInfo
		//{
		//	ShaderType Type = ShaderType::VERTEX_SHADER;
		//	const Shader* Shader = nullptr;
		//};
	};

	class PipelineCache {
	public:
	private:
	};
	
	class GraphicsPipeline : public Pipeline {
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

	class ComputePipeline : public Pipeline {
	public:
	};

	template<class ALLOCATOR>
	std::tuple<bool, GTSL::String<ALLOCATOR>, GTSL::Buffer<ALLOCATOR>> CompileShader2(GTSL::Range<const char8_t*> code, GTSL::Range<const char8_t*> shaderName, ShaderType shaderType, ShaderLanguage shaderLanguage, const ALLOCATOR& allocator) {
		IDxcUtils* pUtils;
		DxcCreateInstance(CLSID_DxcUtils, __uuidof(IDxcUtils), reinterpret_cast<void**>(&pUtils));
		IDxcBlobEncoding* pSource;
		//pUtils->CreateBlob(pShaderSource, shaderSourceSize, CP_UTF8, pSource.GetAddressOf());

		GTSL::Vector<LPWSTR, ALLOCATOR> arguments;
		//-E for the entry point (eg. PSMain)
		arguments.EmplaceBack(L"-E");
		arguments.EmplaceBack(L"main");

		//-T for the target profile (eg. ps_6_2)
		arguments.EmplaceBack(L"-T");

		switch (shaderType) {
		case ShaderType::VERTEX: arguments.EmplaceBack(L"vs_6_5"); break;
		case ShaderType::TESSELLATION_CONTROL: break;
		case ShaderType::TESSELLATION_EVALUATION: break;
		case ShaderType::GEOMETRY: break;
		case ShaderType::FRAGMENT: arguments.EmplaceBack(L"ps_6_5"); break;
		case ShaderType::COMPUTE: arguments.EmplaceBack(L"cs_6_5"); break;
		case ShaderType::TASK: arguments.EmplaceBack(L"ts_6_2"); break;
		case ShaderType::MESH: arguments.EmplaceBack(L"ms_6_2"); break;
		case ShaderType::RAY_GEN: arguments.EmplaceBack(L"lib_6_5"); break;
		case ShaderType::CLOSEST_HIT: arguments.EmplaceBack(L"chs_6_2"); break;
		case ShaderType::ANY_HIT: arguments.EmplaceBack(L"ahs_6_2"); break;
		case ShaderType::INTERSECTION: arguments.EmplaceBack(L"is_6_2"); break;
		case ShaderType::MISS: arguments.EmplaceBack(L"ms_6_2"); break;
		case ShaderType::CALLABLE: break;
		default: ;
		}

		//Strip reflection data and pdbs (see later)
		arguments.EmplaceBack(L"-Qstrip_debug");
		arguments.EmplaceBack(L"-Qstrip_reflect");

		arguments.EmplaceBack(DXC_ARG_WARNINGS_ARE_ERRORS); //-WX
		arguments.EmplaceBack(DXC_ARG_DEBUG); //-Zi
		arguments.EmplaceBack(DXC_ARG_PACK_MATRIX_ROW_MAJOR); //-Zp

		IDxcCompiler3* compiler3;
		DxcCreateInstance(CLSID_DxcCompiler, __uuidof(IDxcCompiler3), reinterpret_cast<void**>(&compiler3));

		IDxcResult* result;

		DxcBuffer dxc_buffer;
		dxc_buffer.Size = code.GetBytes();
		dxc_buffer.Encoding = DXC_CP_UTF8;
		dxc_buffer.Ptr = code.GetData();
		compiler3->Compile(&dxc_buffer, arguments.GetData(), arguments.GetLength(), nullptr, __uuidof(IDxcResult), reinterpret_cast<void**>(&result));
	}

	struct ShaderCompiler {
		ShaderCompiler() {
			shaderc_compile_options.SetTargetSpirv(shaderc_spirv_version_1_5);
			shaderc_compile_options.SetTargetEnvironment(shaderc_target_env_vulkan, shaderc_env_version_vulkan_1_2);
			shaderc_compile_options.SetOptimizationLevel(shaderc_optimization_level_performance);
			shaderc_compile_options.SetGenerateDebugInfo();
		}

		template<class ALLOCATOR>
		std::tuple<bool, GTSL::String<ALLOCATOR>, GTSL::Buffer<ALLOCATOR>> Compile(GTSL::Range<const char8_t*> code, GTSL::Range<const char8_t*> shaderName, ShaderType shaderType, ShaderLanguage shaderLanguage, bool is_debug, const ALLOCATOR& allocator) {
			shaderc_shader_kind shaderc_stage;

			switch (shaderType) {
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

			shaderc_source_language shaderc_source_language = shaderc_source_language_glsl;
			switch (shaderLanguage) {
			case ShaderLanguage::GLSL: shaderc_source_language = shaderc_source_language_glsl; break;
			case ShaderLanguage::HLSL: shaderc_source_language = shaderc_source_language_hlsl; break;
			default: GAL_DEBUG_BREAK;
			}

			shaderc_compile_options.SetSourceLanguage(shaderc_source_language);

			GTSL::String<ALLOCATOR> shaderNameNullTerminator(shaderName, allocator); //String guarantees null terminator, while StringView doesn't and shaderc needs it
			const auto shaderc_module = shaderc_compiler.CompileGlslToSpv(reinterpret_cast<const char*>(code.GetData()), code.GetBytes(), shaderc_stage, reinterpret_cast<const char*>(shaderNameNullTerminator.c_str()), shaderc_compile_options);

			if (shaderc_module.GetCompilationStatus() != shaderc_compilation_status_success) {
				auto errorString = shaderc_module.GetErrorMessage();
				return { false, GTSL::String<ALLOCATOR>{ GTSL::Range(GTSL::Byte(errorString.size()), reinterpret_cast<const char8_t*>(errorString.c_str())), allocator }, GTSL::Buffer<ALLOCATOR>{ allocator } };
			}

			GTSL::Buffer<ALLOCATOR> buffer((shaderc_module.end() - shaderc_module.begin()) * sizeof(GTSL::uint32), 16, allocator);
			buffer.Write((shaderc_module.end() - shaderc_module.begin()) * sizeof(GTSL::uint32), reinterpret_cast<const GTSL::byte*>(shaderc_module.begin()));

			return { true, GTSL::String<ALLOCATOR>{ allocator }, GTSL::Buffer<ALLOCATOR>{ GTSL::MoveRef(buffer) } };
		}

	private:
		shaderc::Compiler shaderc_compiler;
		shaderc::CompileOptions shaderc_compile_options;
	};
}
