#pragma once

#include <GAL/Pipelines.h>
#include <GAL/RenderCore.h>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/String.hpp>
#include <GTSL/File.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/Serialize.hpp>
#include <GTSL/Math/Vectors.hpp>
#include <GTSL/Filesystem.h>
#include <GAL/Serialize.hpp>

#include "ResourceManager.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Render/ShaderGenerator.h"

#include "PermutationManager.hpp"
#include "CommonPermutation.hpp"
#include "ForwardPermutation.hpp"
#include "VisibilityPermutation.hpp"

template<typename T, class A>
auto operator<<(auto& buffer, const GTSL::Vector<T, A>& vector) -> decltype(buffer)& {
	buffer << vector.GetLength();
	for (uint32 i = 0; i < vector.GetLength(); ++i) { buffer << vector[i]; }
	return buffer;
}

template<typename T, class A>
auto operator>>(auto& buffer, GTSL::Vector<T, A>& vector) -> decltype(buffer)& {
	uint32 length;
	buffer >> length;
	for (uint32 i = 0; i < length; ++i) { buffer >> vector.EmplaceBack(); }
	return buffer;
}

template<class A>
auto operator<<(auto& buffer, const GTSL::String<A>& vector) -> decltype(buffer)& {
	buffer << vector.GetBytes() << vector.GetCodepoints();
	for (uint32 i = 0; i < vector.GetBytes(); ++i) { buffer << vector.c_str()[i]; }
	return buffer;
}

template<class A>
auto operator>>(auto& buffer, GTSL::String<A>& vector) -> decltype(buffer)& {
	uint32 length, codepoints;
	buffer >> length >> codepoints;
	for (uint32 i = 0; i < length; ++i) {
		char8_t c;
		buffer >> c;
		vector += c;
	}
	return buffer;
}

template<typename T, class A>
auto Read(auto& buffer, GTSL::Vector<T, A>& vector, const BE::PAR& allocator) -> decltype(buffer)& {
	uint32 length;
	buffer >> length;
	for (uint32 i = 0; i < length; ++i) { Extract(vector.EmplaceBack(), buffer); }
	return buffer;
}

template<uint8 S>
auto operator<<(auto& buffer, const GTSL::ShortString<S>& string) -> decltype(buffer)& {
	for (uint32 i = 0; i < S; ++i) { buffer << string.begin()[i]; }
	return buffer;
}

template<uint8 S>
auto operator>>(auto& buffer, GTSL::ShortString<S>& string) -> decltype(buffer)& {
	for (uint32 i = 0; i < S; ++i) { buffer >> const_cast<char8_t*>(string.begin())[i]; }
	return buffer;
}

template<GTSL::Enum E>
auto operator<<(auto& buffer, const E enu) -> decltype(buffer)& {
	buffer << static_cast<GTSL::UnderlyingType<E>>(enu);
	return buffer;
}

template<GTSL::Enum E>
auto operator>>(auto& buffer, E& enu) -> decltype(buffer)& {
	buffer >> reinterpret_cast<GTSL::UnderlyingType<E>&>(enu);
	return buffer;
}

static unsigned long long quickhash64(const GTSL::Range<const byte*> range) { // set 'mix' to some value other than zero if you want a tagged hash          
	const unsigned long long mulp = 2654435789;
	unsigned long long mix = 0;

	mix ^= 104395301;

	for (auto e : range)
		mix += (e * mulp) ^ (mix >> 23);

	return mix ^ (mix << 37);
}

struct PermutationManager;

class ShaderResourceManager final : public ResourceManager
{
	static GTSL::ShortString<12> ShaderTypeToFileExtension(GAL::ShaderType type) {
		switch (type) {
		case GAL::ShaderType::VERTEX: return u8"vert";
		case GAL::ShaderType::TESSELLATION_CONTROL: return u8"tesc";
		case GAL::ShaderType::TESSELLATION_EVALUATION: return u8"tese";
		case GAL::ShaderType::GEOMETRY: return u8"geom";
		case GAL::ShaderType::FRAGMENT: return u8"frag";
		case GAL::ShaderType::COMPUTE: return u8"comp";
		case GAL::ShaderType::RAY_GEN: return u8"rgen";
		case GAL::ShaderType::ANY_HIT: return u8"rahit";
		case GAL::ShaderType::CLOSEST_HIT: return u8"rchit";
		case GAL::ShaderType::MISS: return u8"rmiss";
		case GAL::ShaderType::INTERSECTION: return u8"rint";
		case GAL::ShaderType::CALLABLE: return u8"rcall";
		}
	}

public:
	static StructElement readStructElement(GTSL::JSONMember json) {
		return { json[u8"type"], json[u8"name"], json[u8"defaultValue"] };
	}

	using ShaderHash = uint64;

	ShaderResourceManager(const InitializeInfo& initialize_info);

	~ShaderResourceManager() = default;

	struct Parameter {
		GTSL::StaticString<32> Type, Name, Value;

		Parameter() = default;
		Parameter(const GTSL::StringView type, const GTSL::StringView name, const GTSL::StringView val) : Type(type), Name(name), Value(val) {}

		template<class ALLOC>
		friend void Insert(const Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer) {
			Insert(parameterInfo.Type, buffer);
			Insert(parameterInfo.Name, buffer);
			Insert(parameterInfo.Value, buffer);
		}

		template<class ALLOC>
		friend void Extract(Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer) {
			Extract(parameterInfo.Type, buffer);
			Extract(parameterInfo.Name, buffer);
			Extract(parameterInfo.Value, buffer);
		}
	};

	struct ShaderGroupInstance {
		ShaderGroupInstance() = default;

		GTSL::ShortString<32> Name;
		GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 16> Parameters;

		ShaderGroupInstance& operator=(const ShaderGroupInstance& shader_group_instance) {
			Name = shader_group_instance.Name; Parameters = shader_group_instance.Parameters;
			return *this;
		}

		template<class ALLOC>
		friend void Insert(const ShaderGroupInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer) {
			Insert(materialInstance.Name, buffer);
			Insert(materialInstance.Parameters, buffer);
		}

		template<class ALLOC>
		friend void Extract(ShaderGroupInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer) {
			Extract(materialInstance.Name, buffer);
			Extract(materialInstance.Parameters, buffer);
		}

	};

	struct VertexShader {};

	struct FragmentShader {
		GAL::BlendOperation WriteOperation;

		template<class ALLOC>
		friend void Insert(const FragmentShader& fragment_shader, GTSL::Buffer<ALLOC>& buffer) {
			Insert(fragment_shader.WriteOperation, buffer);
		}

		template<class ALLOC>
		friend void Extract(FragmentShader& fragment_shader, GTSL::Buffer<ALLOC>& buffer) {
			Extract(fragment_shader.WriteOperation, buffer);
		}
	};

	struct TaskShader {};

	struct MeshShader {};

	struct ComputeShader { GTSL::ShortString<64> Execution; };

	struct RayGenShader { uint8 Recursion = 1; };

	struct ClosestHitShader {};

	struct MissShader {};

	struct AnyHitShader {};

	struct IntersectionShader {};

	struct CallableShader {};

	struct ShaderInfo {
		GTSL::ShortString<32> Name;
		GAL::ShaderType Type; uint64 Hash = 0;
		GTSL::StaticVector<Parameter, 8> Parameters;
		GTSL::StaticVector<GTSL::ShortString<16>, 4> Tags;
		uint32 Size = 0;

		union {
			VertexShader VertexShader;
			FragmentShader FragmentShader;
			ComputeShader ComputeShader;
			TaskShader TaskShader;
			MeshShader MeshShader;
			RayGenShader RayGenShader;
			ClosestHitShader ClosestHitShader;
			MissShader MissShader;
			AnyHitShader AnyHitShader;
			IntersectionShader IntersectionShader;
			CallableShader CallableShader;
		};

		ShaderInfo() {}

		void SetType(GAL::ShaderType type) {
			Type = type;

			switch (Type) {
			case GAL::ShaderType::VERTEX: ::new(&VertexShader) struct VertexShader(); break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: break;
			case GAL::ShaderType::COMPUTE: ::new(&ComputeShader) struct ComputeShader(); break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: ::new(&RayGenShader) struct RayGenShader(); break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			default: __debugbreak();
			}
		}

		ShaderInfo(const ShaderInfo& shader_info) : Name(shader_info.Name), Type(shader_info.Type), Hash(shader_info.Hash), Parameters(shader_info.Parameters), Tags(shader_info.Tags), Size(shader_info.Size) {
			switch (Type) {
			case GAL::ShaderType::VERTEX: ::new(&VertexShader) struct VertexShader(shader_info.VertexShader); break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: ::new(&FragmentShader) struct FragmentShader(shader_info.FragmentShader); break;
			case GAL::ShaderType::COMPUTE: ::new(&ComputeShader) struct ComputeShader(shader_info.ComputeShader); break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: ::new(&RayGenShader) struct RayGenShader(shader_info.RayGenShader); break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			}
		}

		~ShaderInfo() {
			switch (Type) {
			case GAL::ShaderType::VERTEX: GTSL::Destroy(VertexShader); break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: GTSL::Destroy(FragmentShader); break;
			case GAL::ShaderType::COMPUTE: GTSL::Destroy(ComputeShader); break;
			case GAL::ShaderType::TASK: GTSL::Destroy(TaskShader); break;
			case GAL::ShaderType::MESH: GTSL::Destroy(MeshShader); break;
			case GAL::ShaderType::RAY_GEN: GTSL::Destroy(RayGenShader); break;
			case GAL::ShaderType::ANY_HIT: GTSL::Destroy(AnyHitShader); break;
			case GAL::ShaderType::CLOSEST_HIT: GTSL::Destroy(ClosestHitShader); break;
			case GAL::ShaderType::MISS: GTSL::Destroy(MissShader); break;
			case GAL::ShaderType::INTERSECTION: GTSL::Destroy(IntersectionShader); break;
			case GAL::ShaderType::CALLABLE: GTSL::Destroy(CallableShader); break;
			default:;
			}
		}

		ShaderInfo& operator=(const ShaderInfo& other) {
			Size = other.Size;
			Name = other.Name;
			Type = other.Type;
			Hash = other.Hash;
			Parameters = other.Parameters;
			Tags = other.Tags;

			switch (Type) {
			case GAL::ShaderType::VERTEX: VertexShader = other.VertexShader; break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: FragmentShader = other.FragmentShader; break;
			case GAL::ShaderType::COMPUTE: ComputeShader = other.ComputeShader; break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: RayGenShader = other.RayGenShader; break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			}

			return *this;
		}

		template<class ALLOC>
		friend void Insert(const ShaderInfo& shader, GTSL::Buffer<ALLOC>& buffer) {
			Insert(shader.Name, buffer);
			Insert(shader.Type, buffer);
			Insert(shader.Size, buffer);
			Insert(shader.Parameters, buffer);
			Insert(shader.Tags, buffer);

			switch (shader.Type) {
			case GAL::ShaderType::VERTEX: Insert(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Insert(shader.FragmentShader, buffer); break;
			case GAL::ShaderType::COMPUTE: Insert(shader.ComputeShader, buffer); break;
			}
		}

		template<class ALLOC>
		friend void Extract(ShaderInfo& shader, GTSL::Buffer<ALLOC>& buffer) {
			Extract(shader.Name, buffer);
			Extract(shader.Type, buffer);
			Extract(shader.Size, buffer);
			Extract(shader.Parameters, buffer);
			Extract(shader.Tags, buffer);

			switch (shader.Type) {
			case GAL::ShaderType::VERTEX: Extract(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Extract(shader.FragmentShader, buffer); break;
			case GAL::ShaderType::COMPUTE: Extract(shader.ComputeShader, buffer); break;
			}
		}
	};

	struct ShaderGroupData : Data {
		ShaderGroupData(const BE::PAR& allocator) : Parameters(allocator), Instances(allocator), Shaders(allocator) {}

		GTSL::ShortString<32> Name;

		GTSL::Vector<Parameter, BE::PAR> Parameters;
		GTSL::Vector<ShaderGroupInstance, BE::PAR> Instances;
		GTSL::Vector<uint32, BE::PAR> Shaders;
		GTSL::StaticVector<GTSL::StaticVector<StructElement, 8>, 8> VertexElements;
	};

	struct ShaderGroupDataSerialize : ShaderGroupData, Object {
		ShaderGroupDataSerialize(const BE::PAR& allocator) : ShaderGroupData(allocator) {}
	};

	struct ShaderGroupInfo {
		ShaderGroupInfo(const BE::PAR& allocator) : Shaders(allocator), Instances(allocator), Parameters(allocator) {}

		GTSL::ShortString<32> Name;

		GTSL::Vector<ShaderInfo, BE::PAR> Shaders;
		GTSL::Vector<ShaderGroupInstance, BE::PAR> Instances;
		GTSL::Vector<Parameter, BE::PAR> Parameters;

		GTSL::StaticVector<GTSL::StaticVector<StructElement, 8>, 8> VertexElements;

		struct RayTraceData {
			StructElement Payload;

			struct Group {
				GTSL::StaticVector<uint32, 8> ShadersPerGroup;
			} Groups[4];
		} RayTrace;
	};

	template<typename... ARGS>
	void LoadShaderGroupInfo(ApplicationManager* gameInstance, Id shaderGroupName, DynamicTaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"loadShaderInfosFromDisk", {}, &ShaderResourceManager::loadShaderGroup<ARGS...>, {}, {}, GTSL::MoveRef(shaderGroupName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadShaderGroup(ApplicationManager* gameInstance, ShaderGroupInfo shader_group_info, DynamicTaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, GTSL::Range<byte*> buffer, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"loadShadersFromDisk", {}, &ShaderResourceManager::loadShaders<ARGS...>, {}, {}, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File shaderGroupInfosFile, shaderInfosFile, shaderPackageFile;
	GTSL::HashMap<Id, uint64, BE::PersistentAllocatorReference> shaderGroupInfoOffsets;
	GTSL::HashMap<uint64, uint64, BE::PersistentAllocatorReference> shaderInfoOffsets, shaderOffsets;

	mutable GTSL::ReadWriteMutex mutex;

	GAL::ShaderCompiler compiler_;

	template<typename... ARGS>
	void loadShaderGroup(TaskInfo taskInfo, Id shaderGroupName, DynamicTaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS... args) { //TODO: check why can't use ARGS&&
		shaderGroupInfosFile.SetPointer(shaderGroupInfoOffsets[shaderGroupName]);

		ShaderGroupInfo shaderGroupInfo(GetPersistentAllocator());

		shaderGroupInfosFile >> shaderGroupInfo.Name;

		uint32 shaderCount;
		shaderGroupInfosFile >> shaderCount;

		for (uint32 s = 0; s < shaderCount; ++s) {
			uint64 shaderHash;
			shaderGroupInfosFile >> shaderHash;

			auto& shader = shaderGroupInfo.Shaders.EmplaceBack();

			{
				shaderInfosFile.SetPointer(shaderInfoOffsets[shaderHash]);
				shaderInfosFile >> shader.Name >> shader.Size >> shader.Hash;

				uint32 paramCount = 0;
				shaderInfosFile >> paramCount;

				for (uint32 p = 0; p < paramCount; ++p) {
					auto& parameter = shader.Parameters.EmplaceBack();
					shaderInfosFile >> parameter.Name >> parameter.Type >> parameter.Value;
				}

				GAL::ShaderType shaderType;
				shaderInfosFile >> shaderType;

				shader.SetType(shaderType);

				uint32 tagCount = 0;
				shaderInfosFile >> tagCount;

				for(uint32 i = 0; i < tagCount; ++i) {
					shaderInfosFile >> shader.Tags.EmplaceBack();
				}

				GTSL::ShortString<64> execution;

				if(shaderType == GAL::ShaderType::COMPUTE) {
					shaderInfosFile >> execution;
					shader.ComputeShader.Execution = execution;
				}
			}
		}

		uint32 parameterCount;
		shaderGroupInfosFile >> parameterCount;

		for (uint32 p = 0; p < parameterCount; ++p) {
			auto& parameter = shaderGroupInfo.Parameters.EmplaceBack();
			shaderGroupInfosFile >> parameter.Type >> parameter.Name >> parameter.Value;
		}

		uint32 instanceCount;
		shaderGroupInfosFile >> instanceCount;

		for (uint32 i = 0; i < instanceCount; ++i) {
			auto& instance = shaderGroupInfo.Instances.EmplaceBack();
			shaderGroupInfosFile >> instance.Name;

			uint32 params = 0;
			shaderGroupInfosFile >> params;

			for (uint32 p = 0; p < params; ++p) {
				auto& param = instance.Parameters.EmplaceBack();
				shaderGroupInfosFile >> param.First >> param.Second;
			}
		}

		uint32 vertexStreamCount = 0;
		shaderGroupInfosFile >> vertexStreamCount;

		for (uint32 a = 0; a < vertexStreamCount; ++a) {
			auto& stream = shaderGroupInfo.VertexElements.EmplaceBack();

			uint32 vertexElements = 0;
			shaderGroupInfosFile >> vertexElements;

			for (uint32 a = 0; a < vertexElements; ++a) {
				auto& e = stream.EmplaceBack();
				shaderGroupInfosFile >> e.Type >> e.Name;
			}
		}

		bool rayTrace = false; shaderGroupInfosFile >> rayTrace;

		if (rayTrace) {
			shaderGroupInfosFile >> shaderGroupInfo.RayTrace.Payload.Type >> shaderGroupInfo.RayTrace.Payload.Name >> shaderGroupInfo.RayTrace.Payload.DefaultValue;

			for (uint32 i = 0; i < 4; ++i) {
				uint32 groupCount; shaderGroupInfosFile >> groupCount;

				for (uint32 j = 0; j < groupCount; ++j) {
					shaderGroupInfosFile >> shaderGroupInfo.RayTrace.Groups[i].ShadersPerGroup.EmplaceBack();
				}
			}
		}

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(shaderGroupInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadShaders(TaskInfo taskInfo, ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer, DynamicTaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, ARGS... args) {
		uint32 offset = 0;

		for (const auto& s : shader_group_info.Shaders) {
			shaderPackageFile.SetPointer(shaderOffsets[s.Hash]);
			shaderPackageFile.Read(s.Size, offset, buffer);
			offset += s.Size;
		}

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
	};
};

struct VertexPermutationManager {
	VertexPermutationManager(GPipeline* pipeline) {
		for(uint32 i = 0; i < vertexPermutations; ++i) {

			GTSL::StaticVector<StructElement, 8> structElements;

			for(uint32 j = 0; j < vertexPermutations[i]; ++j) {
			}

			vertexPermutationHandles.EmplaceBack(pipeline->DeclareStruct({}, u8"vertex", structElements));
		}
	}

	GTSL::StaticVector<GTSL::StaticVector<GTSL::StaticVector<GAL::ShaderDataType, 8>, 8>, 8> vertexPermutations;
	GTSL::StaticVector<GPipeline::ElementHandle, 8> vertexPermutationHandles;
};

auto constexpr U_STRING = u8"output = instances[output.r].";
auto constexpr F_STRING = u8"vec3f mix(uvec3 indices) { return  }";

struct RayTraceRenderPassPermutation : PermutationManager {
	RayTraceRenderPassPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"RayTraceRenderPassPermutation") {}

	void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		rayTraceHandle = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"RayTraceRenderPassPermutation");

		shader_generation_data.Scopes.EmplaceBack(rayTraceHandle);

		const auto pushConstantBlockHandle = pipeline->DeclareScope(rayTraceHandle, u8"pushConstantBlock");
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"cameraData*", u8"camera" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"rayTraceData*", u8"rayTrace" });

		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

		//auto payloadBlockHandle = pipeline->add(rayTraceHandle, u8"payloadBlock", GPipeline::LanguageElement::ElementType::SCOPE);
		//payloadHandle = pipeline->DeclareVariable(payloadBlockHandle, { u8"vec4f", u8"payload" });
		pipeline->AddMemberDeductionGuide(commonPermutation->closestHitShaderScope, u8"surfaceColor", { payloadHandle });

		shader_generation_data.Hierarchy.EmplaceBack(this);
	}

	void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, GTSL::StaticVector<PermutationManager*, 16> hierarchy, GTSL::StaticVector<Result, 8>& batches) override {
		GTSL::StaticVector<StructElement, 8> shaderParameters;

		//for (auto p : json[u8"parameters"]) {
		//	shaderParameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);
		//}
		//
		//{
		//	auto shaderParametersHandle = pipeline.DeclareStruct(shaderScope, u8"shaderParametersData", shaderParameters);
		//
		//	for (auto& e : shaderParameters) {
		//		pipeline.AddMemberDeductionGuide(shaderScope, shaderParameters.back().Name, { shaderRecordEntry, pipeline.GetElementHandle(shaderParametersHandle, e.Name) });
		//	}
		//}
		//
		bool transparency = false;
		
		if (auto tr = shader_json[u8"transparency"]) {
			transparency = tr.GetBool();
		}

		if (transparency) {
			auto e = batches.EmplaceBack();
			e.TargetSemantics = GAL::ShaderType::ANY_HIT;
		} else {
			////translate writes to pixelColor variable into Write() calls for ray gen shader 
			//[&]() {
			//	for (uint32 l = 0; l < pipeline.GetFunction(shaderMainHandle).Statements; ++l) {
			//		auto& s = main.Statements[l];
			//		for (uint32 i = 0; i < s; ++i) {
			//			if (s[i].Name == u8"pixelColor") {
			//				if (s[i + 1].Name == u8"=") {
			//					auto& newStatement = main.Statements.EmplaceBack();
			//					newStatement.EmplaceBack(ShaderNode::Type::ID, u8"Write");
			//					newStatement.EmplaceBack(ShaderNode::Type::LPAREN, u8"(");
			//					newStatement.EmplaceBack(ShaderNode::Type::ID, u8"pushConstantBlock");
			//					newStatement.EmplaceBack(ShaderNode::Type::DOT, u8".");
			//					newStatement.EmplaceBack(ShaderNode::Type::ID, u8"renderPass");
			//					newStatement.EmplaceBack(ShaderNode::Type::DOT, u8".");
			//					newStatement.EmplaceBack(ShaderNode::Type::ID, u8"Color");
			//					newStatement.EmplaceBack(ShaderNode::Type::COMMA, u8",");
			//					newStatement.EmplaceBack(ShaderNode::Type::ID, u8"GetFragmentPosition"); newStatement.EmplaceBack(ShaderNode::Type::LPAREN, u8"(");	newStatement.EmplaceBack(ShaderNode::Type::RPAREN, u8")");
			//					newStatement.EmplaceBack(ShaderNode::Type::COMMA, u8",");
			//
			//					for (uint32 j = i + 2; j < s; ++j) {
			//						newStatement.EmplaceBack(s[j]);
			//					}
			//
			//					newStatement.EmplaceBack(ShaderNode::Type::RPAREN, u8")");
			//					main.Statements.Pop(l);
			//					return;
			//				}
			//			}
			//		}
			//	}
			//}();

			auto e = batches.EmplaceBack();
			e.TargetSemantics = GAL::ShaderType::CLOSEST_HIT;
		}
	}

	GPipeline::ElementHandle rayTraceHandle, payloadHandle;
};

inline ShaderResourceManager::ShaderResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"ShaderResourceManager"), shaderGroupInfoOffsets(8, GetPersistentAllocator()), shaderInfoOffsets(8, GetPersistentAllocator()), shaderOffsets(8, GetPersistentAllocator()) {
	shaderPackageFile.Open(GetResourcePath(u8"Shaders", u8"bepkg"), GTSL::File::READ | GTSL::File::WRITE, true);

	GTSL::File shaderGroupsTableFile, shaderInfoTableFile, shadersTableFile;
	shaderGroupsTableFile.Open(GetResourcePath(u8"ShaderGroups.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);
	shaderInfoTableFile.Open(GetResourcePath(u8"ShaderInfo.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);
	shadersTableFile.Open(GetResourcePath(u8"Shaders.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);

	bool created = false;

	switch (shaderInfosFile.Open(GetResourcePath(u8"Shaders", u8"beidx"), GTSL::File::READ | GTSL::File::WRITE, true)) {
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	}

	switch (shaderGroupInfosFile.Open(GetResourcePath(u8"ShaderGroups", u8"beidx"), GTSL::File::READ | GTSL::File::WRITE, true)) {
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	}

	if (!(shaderPackageFile.GetSize() && shaderGroupsTableFile.GetSize() && shaderInfoTableFile.GetSize() && shadersTableFile.GetSize() && shaderInfosFile.GetSize() && shaderGroupInfosFile.GetSize())) {
		shaderPackageFile.Resize(0);
		shaderGroupsTableFile.Resize(0);
		shaderInfoTableFile.Resize(0);
		shadersTableFile.Resize(0);
		shaderInfosFile.Resize(0);
		shaderGroupInfosFile.Resize(0);
		created = true;
	}

	if (created) {
		GTSL::KeyMap<ShaderHash, BE::TAR> loadedShaders(128, GetTransientAllocator());

		GTSL::FileQuery shaderGroupFileQuery;

		while (auto fileRef = shaderGroupFileQuery.DoQuery(GetResourcePath(u8"*ShaderGroup.json"))) {
			GTSL::File shaderGroupFile; shaderGroupFile.Open(GetResourcePath(fileRef.Get()), GTSL::File::READ, false);

			GTSL::Buffer buffer(shaderGroupFile.GetSize(), 16, GetTransientAllocator()); shaderGroupFile.Read(buffer);

			GTSL::SmartPointer<CommonPermutation, BE::TAR> commonPermutation(GetTransientAllocator(), u8"Common");

			{ //configure permutations
				commonPermutation->CreateChild<ForwardRenderPassPermutation>(u8"ForwardRenderPassPermutation");
				commonPermutation->CreateChild<VisibilityRenderPassPermutation>(u8"VisibilityRenderPassPermutation");
			} //todo: parametrize

			GTSL::Buffer deserializer(GetTransientAllocator());
			auto json = Parse(GTSL::StringView(GTSL::Byte(buffer.GetLength()), reinterpret_cast<const utf8*>(buffer.GetData())), deserializer);

			ShaderGroupDataSerialize shaderGroupDataSerialize(GetPersistentAllocator());
			shaderGroupDataSerialize.Name = json[u8"name"];

			GPipeline pipeline;

			PermutationManager::InitializePermutations(commonPermutation, &pipeline);

			GTSL::StaticVector<uint64, 16> shaderGroupUsedShaders;

			if (auto structs = json[u8"structs"]) {
				for (auto s : structs) {
					GTSL::StaticVector<StructElement, 8> elements;

					for (auto m : s[u8"members"]) {
						elements.EmplaceBack(m[u8"type"], m[u8"name"]);
					}

					pipeline.DeclareStruct(GPipeline::ElementHandle(), s[u8"name"], elements);
				}
			}

			if (auto functions = json[u8"functions"]) {
				for (auto f : functions) {
					GTSL::StaticVector<StructElement, 8> elements;
					for (auto p : f[u8"params"]) { elements.EmplaceBack(p[u8"type"], p[u8"name"]); }
					pipeline.DeclareFunction(GPipeline::ElementHandle(), f[u8"type"], f[u8"name"], elements, f[u8"code"]);
				}
			}

			for (auto i : json[u8"instances"]) {
				auto& instance = shaderGroupDataSerialize.Instances.EmplaceBack();
				instance.Name = i[u8"name"];

				for (auto f : i[u8"parameters"]) {
					auto& param = instance.Parameters.EmplaceBack();
					param.First = f[u8"name"];
					param.Second = f[u8"defaultValue"];
				}
			}

			if (auto parameters = json[u8"parameters"]) {
				for (auto p : parameters) {
					if (auto def = p[u8"defaultValue"]) {
						shaderGroupDataSerialize.Parameters.EmplaceBack(p[u8"type"], p[u8"name"], def);
					} else {
						shaderGroupDataSerialize.Parameters.EmplaceBack(p[u8"type"], p[u8"name"], u8"");
					}
				}
			}

			shaderGroupDataSerialize.VertexElements.EmplaceBack().EmplaceBack(u8"vec3f", u8"POSITION");
			shaderGroupDataSerialize.VertexElements.EmplaceBack().EmplaceBack(u8"vec3f", u8"NORMAL");
			shaderGroupDataSerialize.VertexElements.EmplaceBack().EmplaceBack(u8"vec3f", u8"TANGENT");
			shaderGroupDataSerialize.VertexElements.EmplaceBack().EmplaceBack(u8"vec3f", u8"BITANGENT");
			shaderGroupDataSerialize.VertexElements.EmplaceBack().EmplaceBack(u8"vec2f", u8"TEXTURE_COORDINATES");

			bool rayTrace = true; ShaderGroupInfo::RayTraceData ray_trace_data;

			for(auto s : json[u8"shaders"]) {
				GTSL::File shaderFile; shaderFile.Open(GetResourcePath(s[u8"name"], u8"json"));
				GTSL::Buffer shaderFileBuffer(shaderFile.GetSize(), 16, GetTransientAllocator()); shaderFile.Read(shaderFileBuffer);

				GTSL::Buffer json_deserializer(BE::TAR(u8"GenerateShader"));
				auto shaderJson = Parse(GTSL::StringView(GTSL::Byte(shaderFileBuffer.GetLength()), reinterpret_cast<const utf8*>(shaderFileBuffer.GetData())), json_deserializer);

				Class shaderClass;

				switch (GTSL::Hash(shaderJson[u8"class"])) {
				case GTSL::Hash(u8"Vertex"): shaderClass = Class::VERTEX; break;
				case GTSL::Hash(u8"Surface"): shaderClass = Class::SURFACE; break;
				case GTSL::Hash(u8"Compute"): shaderClass = Class::COMPUTE; break;
				case GTSL::Hash(u8"RayGen"): shaderClass = Class::RAY_GEN; break;
				case GTSL::Hash(u8"Miss"): shaderClass = Class::MISS; break;
				}

				auto shaderBatch = PermutationManager::ProcessShaders(commonPermutation, &pipeline, json, shaderJson);

				GTSL::ShortString<64> executionString;

				if(auto execution = shaderJson[u8"execution"]) {
					executionString = execution;
				}

				if (auto domain = json[u8"domain"]; domain.GetStringView() == u8"Screen") {
					executionString = GTSL::ShortString<64>(u8"windowExtent");
				}

				for (auto& sb : shaderBatch) {
					auto shaderResult = GenerateShader(pipeline, sb.Scopes, sb.TargetSemantics, GetTransientAllocator());
					if (!shaderResult) { BE_LOG_WARNING(shaderResult.Get().Second); }
					auto shaderHash = quickhash64(GTSL::Range(shaderResult.Get().First.GetBytes(), reinterpret_cast<const byte*>(shaderResult.Get().First.c_str())));

					if (!loadedShaders.Find(shaderHash)) {
						loadedShaders.Emplace(shaderHash);

						GTSL::StaticString<512> shaderName;

						//make shader name by appending all the names of the scopes that comprise, which allows to easily identify the permutation
						for(auto& e : sb.Scopes) {
							auto& n = pipeline.GetElement(e).Name;

							if(n) {
								if (shaderName.GetBytes()) {
									shaderName += u8".";
								}

								shaderName += n;
							}
						}

						auto [compRes, resultString, shaderBuffer] = compiler_.Compile(shaderResult.Get().First, shaderName, sb.TargetSemantics, GAL::ShaderLanguage::GLSL, true, GetTransientAllocator());

						if (!compRes) { BE_LOG_ERROR(shaderResult.Get().First); BE_LOG_ERROR(resultString); }

						shaderInfoTableFile << shaderHash << shaderInfosFile.GetSize(); //shader info table
						shadersTableFile << shaderHash << shaderPackageFile.GetSize(); //shader table

						shaderInfosFile << GTSL::ShortString<32>(s[u8"name"]) << static_cast<uint32>(shaderBuffer.GetLength()) << shaderHash;
						shaderInfosFile << 0; //0 params
						shaderInfosFile << sb.TargetSemantics;
						shaderInfosFile << sb.Tags.GetLength();
						for(auto& e : sb.Tags) {
							shaderInfosFile << e;
						}

						if (sb.TargetSemantics == GAL::ShaderType::COMPUTE) {
							shaderInfosFile << executionString;
						}

						shaderPackageFile.Write(shaderBuffer);
					}

					shaderGroupDataSerialize.Shaders.EmplaceBack(shaderGroupUsedShaders.GetLength());
					shaderGroupUsedShaders.EmplaceBack(shaderHash);
				}
			}

			shaderGroupsTableFile << shaderGroupDataSerialize.Name << shaderGroupInfosFile.GetSize();

			{
				shaderGroupInfosFile << shaderGroupDataSerialize.Name;

				shaderGroupInfosFile << shaderGroupUsedShaders.GetLength();
				for (auto& e : shaderGroupUsedShaders) { shaderGroupInfosFile << e; }

				shaderGroupInfosFile << shaderGroupDataSerialize.Parameters.GetLength();
				for (auto& p : shaderGroupDataSerialize.Parameters) {
					shaderGroupInfosFile << p.Type << p.Name << p.Value;
				}

				shaderGroupInfosFile << shaderGroupDataSerialize.Instances.GetLength();
				for (auto& i : shaderGroupDataSerialize.Instances) {
					shaderGroupInfosFile << i.Name;

					shaderGroupInfosFile << i.Parameters.GetLength();
					for (auto& p : i.Parameters) {
						shaderGroupInfosFile << p.First << p.Second;
					}
				}


				shaderGroupInfosFile << shaderGroupDataSerialize.VertexElements.GetLength();
				for (auto& e : shaderGroupDataSerialize.VertexElements) {
					shaderGroupInfosFile << e.GetLength();
					for (auto& ve : e) {
						shaderGroupInfosFile << ve.Type << ve.Name;
					}
				}

				shaderGroupInfosFile << rayTrace;

				if (rayTrace) {
					shaderGroupInfosFile << ray_trace_data.Payload.Type << ray_trace_data.Payload.Name << ray_trace_data.Payload.DefaultValue;

					for (uint32 i = 0; i < 4; ++i) {
						shaderGroupInfosFile << ray_trace_data.Groups[i].ShadersPerGroup.GetLength();

						for (uint32 j = 0; j < ray_trace_data.Groups[i].ShadersPerGroup.GetLength(); ++j) {
							shaderGroupInfosFile << ray_trace_data.Groups[i].ShadersPerGroup[j];
						}
					}
				}
			}
		}
	}

	shaderGroupsTableFile.SetPointer(0);
	{
		uint32 offset = 0;
		while (offset < shaderGroupsTableFile.GetSize()) {
			GTSL::ShortString<32> name; uint64 position;
			shaderGroupsTableFile >> name >> position;
			offset += 32 + 8;
			shaderGroupInfoOffsets.Emplace(Id(name), position);
		}
	}

	shaderInfoTableFile.SetPointer(0);
	{
		uint32 offset = 0;
		while (offset < shaderInfoTableFile.GetSize()) {
			uint64 name; uint64 position;
			shaderInfoTableFile >> name >> position;
			offset += 8 + 8;
			shaderInfoOffsets.Emplace(name, position);
		}
	}

	shadersTableFile.SetPointer(0);
	{
		uint32 offset = 0;
		while (offset < shadersTableFile.GetSize()) {
			uint64 name; uint64 position;
			shadersTableFile >> name >> position;
			offset += 8 + 8;
			shaderOffsets.Emplace(name, position);
		}
	}
}