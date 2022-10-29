#pragma once

#include <GAL/Pipelines.h>
#include <GAL/RenderCore.h>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/SmartPointer.hpp>
#include <GTSL/String.hpp>
#include <GTSL/File.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/Serialize.hpp>
#include <GTSL/Filesystem.h>
#include <GAL/Serialize.hpp>

#include "ResourceManager.h"
#include "ShaderCompilation.hpp"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Render/ShaderGenerator.h"
#include "ByteEngine/Graph.hpp"

#include "PermutationManager.hpp"
#include "CommonPermutation.hpp"
#include "ForwardPermutation.hpp"
#include "UIPermutation.hpp"
#include "RayTracePermutation.hpp"
#include "GTSL/JSON.hpp"
#include "GTSL/Mutex.h"

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

template<typename T, class A>
auto Read(auto& buffer, GTSL::Vector<T, A>& vector, const BE::PAR& allocator) -> decltype(buffer)& {
	uint32 length;
	buffer >> length;
	for (uint32 i = 0; i < length; ++i) { Extract(vector.EmplaceBack(), buffer); }
	return buffer;
}

template<uint8 S>
auto operator>>(auto& buffer, GTSL::ShortString<S>& string) -> decltype(buffer)& {
	uint32 length, codepoints;
	buffer >> length; buffer >> codepoints;
	for (uint32 i = 0; i < length; ++i) { buffer >> const_cast<char8_t*>(string.begin())[i]; }
	return buffer;
}

template<class A>
A& operator<<(A& buffer, GTSL::Enum auto enu) {
	buffer << static_cast<GTSL::UnderlyingType<decltype(enu)>>(enu);
	return buffer;
}

template<class A, GTSL::Enum T>
A& operator>>(A& buffer, T& enu) {
	buffer >> reinterpret_cast<GTSL::UnderlyingType<T>&>(enu);
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
	static StructElement readStructElement(GTSL::JSON<BE::PAR> json) {
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

	struct ComputeShader {};

	struct RayGenShader { uint8 Recursion = 1; };

	struct ClosestHitShader {};

	struct MissShader {};

	struct AnyHitShader {};

	struct IntersectionShader {};

	struct CallableShader {};

	struct ShaderInfo {
		GTSL::ShortString<64> Name;
		GAL::ShaderType Type; uint64 Hash = 0;
		GTSL::StaticVector<Parameter, 8> Parameters;
		GTSL::StaticVector<PermutationManager::ShaderTag, 4> Tags;
		GTSL::StaticString<4096> DebugData;

		uint64 Size = 0;

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

		//ShaderInfo(const BE::PAR& allocator) : DebugData(allocator) {}
		ShaderInfo(const BE::PAR& allocator) {}

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

		ShaderInfo(const ShaderInfo& shader_info) : Name(shader_info.Name), Type(shader_info.Type), Hash(shader_info.Hash), Parameters(shader_info.Parameters), Tags(shader_info.Tags), DebugData(shader_info.DebugData), Size(shader_info.Size) {
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
			DebugData = other.DebugData;

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
			Insert(shader.DebugData, buffer);

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
			Extract(shader.DebugData, buffer);

			switch (shader.Type) {
			case GAL::ShaderType::VERTEX: Extract(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Extract(shader.FragmentShader, buffer); break;
			case GAL::ShaderType::COMPUTE: Extract(shader.ComputeShader, buffer); break;
			}
		}
	};

	struct ShaderGroupData : Data {
		ShaderGroupData(const BE::PAR& allocator) : Parameters(allocator), Instances(allocator), Shaders(allocator), Tags(allocator) {}
	
		GTSL::ShortString<32> Name;
	
		GTSL::Vector<Parameter, BE::PAR> Parameters;
		GTSL::Vector<ShaderGroupInstance, BE::PAR> Instances;
		GTSL::Vector<uint64, BE::PAR> Shaders;
		GTSL::Vector<PermutationManager::ShaderTag, BE::PAR> Tags;
		GTSL::StaticVector<GTSL::StaticVector<StructElement, 8>, 8> VertexElements;
	};

	struct ShaderGroupInfo {
		ShaderGroupInfo(const BE::PAR& allocator) : Shaders(allocator), Instances(allocator), Parameters(allocator), Tags(allocator) {}

		ShaderGroupInfo(ShaderGroupInfo&&) = default;

		GTSL::ShortString<32> Name;

		GTSL::Vector<ShaderInfo, BE::PAR> Shaders;
		GTSL::Vector<ShaderGroupInstance, BE::PAR> Instances;
		GTSL::Vector<Parameter, BE::PAR> Parameters;
		GTSL::Vector<PermutationManager::ShaderTag, BE::PAR> Tags;
		GTSL::StaticVector<GTSL::StaticVector<StructElement, 8>, 8> VertexElements;

		struct RayTraceData {
			StructElement Payload;

			struct Group {
				GTSL::StaticVector<uint32, 8> ShadersPerGroup;
			} Groups[4];
		} RayTrace;
	};

	struct ShaderGroupDataSerialize : ShaderGroupData, Object {
		ShaderGroupDataSerialize(const BE::PAR& allocator) : ShaderGroupData(allocator) {}
		ShaderGroupDataSerialize(const ShaderGroupDataSerialize& other) = delete;
		ShaderGroupDataSerialize(ShaderGroupDataSerialize&&) = default;
		bool RayTrace;
		ShaderGroupInfo::RayTraceData RayTraceData;
		GTSL::StaticVector<GTSL::StaticVector<GTSL::StaticVector<GPipeline::ElementHandle, 8>, 8>, 8> SSS;
		GTSL::StaticVector<GTSL::JSON<BE::TAR>, 8> ShaderJSONs;
	};

	template<typename... ARGS>
	void LoadShaderGroupInfo(ApplicationManager* gameInstance, Id shaderGroupName, TaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->EnqueueTask(gameInstance->RegisterTask(this, u8"loadShaderInfosFromDisk", {}, &ShaderResourceManager::loadShaderGroup<ARGS...>), GTSL::MoveRef(shaderGroupName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadShaderGroup(ApplicationManager* gameInstance, ShaderGroupInfo shader_group_info, TaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, GTSL::Range<byte*> buffer, ARGS&&... args) {
		gameInstance->EnqueueTask(gameInstance->RegisterTask(this, u8"loadShadersFromDisk", {}, &ShaderResourceManager::loadShaders<ARGS...>), GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File shaderGroupInfosFile, shaderInfosFile, shaderPackageFile, changeCache;
	GTSL::HashMap<Id, uint64, BE::PersistentAllocatorReference> shaderGroupInfoOffsets;
	GTSL::HashMap<uint64, uint64, BE::PersistentAllocatorReference> shaderInfoOffsets, shaderOffsets, shaderInfoPointers, shadersPointer;

	mutable GTSL::ReadWriteMutex mutex;

	GAL::ShaderCompiler compiler_;

	using ShaderMap = GTSL::HashMap<GTSL::StringView, GTSL::Tuple<GTSL::JSON<BE::PAR>, GTSL::String<BE::PAR>>, BE::TAR>;

	void processShaderGroup(const GTSL::JSON<BE::PAR>& json, GPipeline& pipeline, PermutationManager* root_permutation, ShaderGroupDataSerialize*, const ShaderMap& shader_map);
	void makeShaderGroup(const GTSL::JSON<BE::PAR>& json, GPipeline& pipeline, PermutationManager* root_permutation, ShaderGroupDataSerialize*, const ShaderMap& shader_map);

	void serializeShaderGroup(const ShaderGroupDataSerialize& shader_group_data_serialize);

	GTSL::Buffer<BE::TransientAllocatorReference> compileShader(const GTSL::JSON<BE::TAR>& json,
	                                                            const GPipeline& pipeline,
	                                                            const GTSL::Range<const GPipeline::ElementHandle*>
	                                                            scopes);
	void serializeShader(const GTSL::JSON<BE::TAR>& json, GTSL::Range<const byte*> shader_binary_buffer);

	template<typename... ARGS>
	void loadShaderGroup(TaskInfo taskInfo, Id shaderGroupName, TaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS... args) { //TODO: check why can't use ARGS&&
		if(!shaderGroupInfoOffsets.Find(shaderGroupName)) { BE_LOG_WARNING(u8"Shader group: ", GTSL::StringView(shaderGroupName), u8", does not exist."); return; }

		shaderGroupInfosFile.SetPointer(shaderGroupInfoOffsets[shaderGroupName]);

		ShaderGroupInfo shaderGroupInfo(GetPersistentAllocator());

		shaderGroupInfosFile >> shaderGroupInfo.Name;

		uint32 shaderCount;
		shaderGroupInfosFile >> shaderCount;

		for (uint32 s = 0; s < shaderCount; ++s) {
			uint64 shaderHash;
			shaderGroupInfosFile >> shaderHash;

			auto& shader = shaderGroupInfo.Shaders.EmplaceBack(GetPersistentAllocator());

			{
				shaderInfosFile.SetPointer(shaderInfoOffsets[shaderHash]);

				GTSL::String jsonString(GetTransientAllocator());

				shaderInfosFile >> jsonString;

				auto shaderJSON = GTSL::JSON(jsonString, GetTransientAllocator());

				shader.Name = shaderJSON[u8"name"];
				shader.Size = uint64(shaderJSON[u8"binarySize"]);
				shader.Hash = uint64(shaderJSON[u8"binaryHash"]);

				// TODO: params

				shader.SetType(ShaderTypeFromString(shaderJSON[u8"targetSemantics"]));

				for(auto tag : shaderJSON[u8"tags"]) {
					shader.Tags.EmplaceBack(GTSL::StringView(tag[u8"name"]), GTSL::StringView(tag[u8"value"]));
				}

				//shaderInfosFile >> shader.DebugData;
			}
		}

		{
			uint32 parameterCount;
			shaderGroupInfosFile >> parameterCount;

			for (uint32 p = 0; p < parameterCount; ++p) {
				auto& parameter = shaderGroupInfo.Parameters.EmplaceBack();
				shaderGroupInfosFile >> parameter.Type >> parameter.Name >> parameter.Value;
			}
		}

		{
			uint32 tagCount;
			shaderGroupInfosFile >> tagCount;

			for (uint32 p = 0; p < tagCount; ++p) {
				auto& tag = shaderGroupInfo.Tags.EmplaceBack();
				shaderGroupInfosFile >> tag.First >> tag.Second;
			}
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

		taskInfo.ApplicationManager->EnqueueTask(dynamicTaskHandle, GTSL::MoveRef(shaderGroupInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadShaders(TaskInfo taskInfo, ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer, TaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, ARGS... args) {
		uint32 offset = 0;

		for (const auto& s : shader_group_info.Shaders) {
			shaderPackageFile.SetPointer(shaderOffsets[GTSL::Hash(s.Name)]);
			shaderPackageFile.Read(s.Size, buffer.begin() + offset);
			offset += s.Size;
		}

		taskInfo.ApplicationManager->EnqueueTask(dynamicTaskHandle, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
	}

	using Result = GTSL::StaticVector<PermutationManager::ShaderPermutation, 8>;

	GTSL::File shaderGroupsTableFile, shaderInfoTableFile, shadersTableFile;
};

struct VertexPermutationManager {
	VertexPermutationManager(GPipeline* pipeline) {
		for(uint32 i = 0; i < vertexPermutations; ++i) {

			GTSL::StaticVector<StructElement, 8> structElements;

			for(uint32 j = 0; j < vertexPermutations[i]; ++j) {
			}

			//vertexPermutationHandles.EmplaceBack(pipeline->DeclareStruct({}, u8"vertex", structElements));
		}
	}

	GTSL::StaticVector<GTSL::StaticVector<GTSL::StaticVector<GAL::ShaderDataType, 8>, 8>, 8> vertexPermutations;
	GTSL::StaticVector<GPipeline::ElementHandle, 8> vertexPermutationHandles;
};

inline ShaderResourceManager::ShaderResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"ShaderResourceManager"), shaderGroupInfoOffsets(8, GetPersistentAllocator()), shaderInfoOffsets(8, GetPersistentAllocator()), shaderOffsets(8, GetPersistentAllocator()), shaderInfoPointers(8, GetPersistentAllocator()), shadersPointer(8, GetPersistentAllocator()) {
	auto a0 = shaderPackageFile.Open(GetResourcePath(u8"Shaders", u8"bepkg"), GTSL::File::READ | GTSL::File::WRITE, true);

	auto a1 = shaderGroupsTableFile.Open(GetResourcePath(u8"ShaderGroups.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);
	auto a2 = shaderInfoTableFile.Open(GetResourcePath(u8"ShaderInfo.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);
	auto a3 = shadersTableFile.Open(GetResourcePath(u8"Shaders.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);

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

	GTSL::HashMap<GTSL::StringView, PermutationManager*, BE::TAR> permutations(8, GetTransientAllocator());

	GTSL::SmartPointer<CommonPermutation, BE::TAR> commonPermutation(GetTransientAllocator(), u8"CommonPermutation");

	permutations.Emplace(u8"CommonPermutation", static_cast<PermutationManager*>(commonPermutation.GetData()));


	{ //configure permutations
		permutations.Emplace(u8"ForwardRenderingPermutation", commonPermutation->CreateChild<ForwardRenderPassPermutation>(u8"ForwardRenderingPermutation"));
		permutations.Emplace(u8"RayTracePermutation", commonPermutation->CreateChild<RayTracePermutation>(u8"RayTracePermutation"));
		permutations.Emplace(u8"UIPermutation", commonPermutation->CreateChild<UIPermutation>(u8"UIPermutation"));
	}

	GPipeline pipeline;

	PermutationManager::InitializePermutations(commonPermutation, &pipeline);

	auto a4 = changeCache.Open(GetResourcePath(u8"ShaderResourceManagerCache.bin"), GTSL::File::READ | GTSL::File::WRITE, true);

	auto changedFiles = GetChangedFiles(GetTransientAllocator(), changeCache, { GetUserResourcePath(u8"*.bespg.json"), GetUserResourcePath(u8"*.besh.json"), GetUserResourcePath(u8"*.besg.json"), GetUserResourcePath(u8"*.besh.txt") });

	if (!(shaderPackageFile.GetSize() && shaderGroupsTableFile.GetSize() && shaderInfoTableFile.GetSize() && shadersTableFile.GetSize() && shaderInfosFile.GetSize() && shaderGroupInfosFile.GetSize() && changeCache.GetSize())) {
		shaderPackageFile.Resize(0);
		shaderGroupsTableFile.Resize(0);
		shaderInfoTableFile.Resize(0);
		shadersTableFile.Resize(0);
		shaderInfosFile.Resize(0);
		shaderGroupInfosFile.Resize(0);
	}

	ShaderMap shaderMap(32, GetTransientAllocator());
	GTSL::HashMap<GTSL::StringView, ShaderGroupDataSerialize, BE::TAR> shaderGroupMap(32, GetTransientAllocator());
	GTSL::HashMap<GTSL::StringView, GTSL::JSON<BE::PAR>, BE::TAR> shaderGroupJSONsMap(32, GetTransientAllocator());

	auto fileTree = GetTree(GetTransientAllocator(), changeCache);

	struct IFile {
		GTSL::StaticString<128> Name;
		uint64 FileHash = 0;
		uint64 Pointer = 0ull;
		State State;
	};

	GTSL::HashMap<uint64, Graph<IFile>, BE::TAR> dependencyTree(128, GetTransientAllocator());

	for(auto& e : fileTree) { // Add files from the cache to the map
		dependencyTree.Emplace(e.GetData().FileNameHash, IFile{ GTSL::StringView(e.GetData().Name), e.GetData().FileHash, e.GetData().Pointer, e.GetData().State });
	}

	// Add new files to the map and update properties for files added from the cache.
	for (auto& e : changedFiles) {
		switch (e.State) {
		case State::ADDED: BE_LOG_MESSAGE(u8"Created ", e.Name); dependencyTree.Emplace(GTSL::Hash(e.Name), IFile{ GTSL::StaticString<128>(e.Name), e.FileHash, 0ull, e.State }); break;
		case State::MODIFIED: BE_LOG_MESSAGE(u8"Modified ", e.Name);
			dependencyTree[GTSL::Hash(e.Name)].GetData().FileHash = e.FileHash;
			dependencyTree[GTSL::Hash(e.Name)].GetData().State = e.State;
			break;
		case State::DELETED: BE_LOG_MESSAGE(u8"Deleted ", e.Name); break;
		}
	}

	// Reads all files and populates all needed structures to compile shaders
	auto populateTree = [&](Graph<IFile>& node, State state) -> void {
		auto& fileData = node.GetData();

		if(state == State::ADDED) { // Only add file to cache if it's the first time we see it.
			fileData.Pointer = CommitFileChangeToCache(changeCache, fileData.Name, fileData.FileHash, 0ull);
		}

		if(GTSL::IsIn(fileData.Name, u8"bespg")) {
			GTSL::File file(GetUserResourcePath(fileData.Name));

			GTSL::StaticBuffer<2048> fileBuffer(file);

			auto json = GTSL::JSON(GTSL::StringView(fileBuffer), GetPersistentAllocator());

			auto permutation = permutations[GTSL::StringView(json[u8"name"])];

			auto permutationScopeHandle = pipeline.TryDeclareScope(GPipeline::GLOBAL_SCOPE, GTSL::StringView(json[u8"name"]));

			if(auto inherits = json[u8"inherits"]) {
				for(auto de : inherits) { // Connect parent permutation guides to their children
					auto& parentDependency = dependencyTree[GTSL::Hash(GTSL::StaticString<128>(de) + u8".bespg.json")];
					parentDependency.Connect(node);
				}
			}

			if(auto dataLayers = json[u8"dataLayers"]) {
				auto pcd = pipeline.DeclareScope(permutationScopeHandle, u8"pushConstantBlock");

				for(auto dataLayer : dataLayers) {
					pipeline.DeclareVariable(pcd, { dataLayer[u8"type"], dataLayer[u8"name"] });
				}
			}

			for(auto option : json[u8"options"]) {
				auto& a = permutation->a.EmplaceBack(GetPersistentAllocator());

				permutation->AddSupportedDomain(option[u8"domain"]);

				for(auto subset : option[u8"subsets"]) {
					GTSL::StaticString<128> subsetPath;
					subsetPath += GTSL::Join { { GTSL::StringView(json[u8"name"]), GTSL::StringView(option[u8"name"]), GTSL::StringView(subset[u8"name"]) }, u8"." };
					GTSL::File guideSubsetShaderTemplateFile(GetUserResourcePath(subsetPath, u8"besh.txt"));

					node.Connect(dependencyTree[GTSL::Hash(subsetPath + u8".besh.txt")]); // Connect permutation guide to shader template

					if(guideSubsetShaderTemplateFile) {
						a.EmplaceBack(guideSubsetShaderTemplateFile, GetPersistentAllocator());
					} else {
						BE_LOG_WARNING(u8"Didn't find any shader subset template for subset: ", GTSL::StringView(subset[u8"name"]), u8".");
					}
				}
			}

			permutation->JSON = GTSL::MoveRef(json);
		}

		if(GTSL::IsIn(fileData.Name, u8"besh.json")) {
			GTSL::File jsonShaderFile(GetUserResourcePath(fileData.Name));
			GTSL::StaticBuffer<2048> jsonShaderFileBuffer(jsonShaderFile);
			
			auto json = GTSL::JSON(GTSL::StringView(jsonShaderFileBuffer), GetPersistentAllocator());

			auto& shaderEntry = shaderMap.Emplace(json[u8"name"], GetPersistentAllocator(), GetPersistentAllocator());

			if(auto code = json[u8"code"]) {
				shaderEntry.rest.element = GTSL::StringView(code);
			} else {
				GTSL::File shaderFile(GetUserResourcePath(json[u8"name"], u8"besh.txt"));

				if(shaderFile) {
					GTSL::Buffer<BE::TAR> shaderFileBuffer(shaderFile, GetTransientAllocator());
					shaderEntry.rest.element = GTSL::StringView(shaderFileBuffer);
				} else {
					BE_LOG_WARNING(u8"Did not find a shader file for shader: ", json[u8"name"], u8".");
				}
			}

			shaderEntry.element = GTSL::MoveRef(json);
		}

		if(GTSL::IsIn(fileData.Name, u8"besg")) {
			auto filePath = fileData.Name; RTrimFirst(filePath, u8'.');

			ShaderGroupDataSerialize& shaderGroupDataSerialize = shaderGroupMap.Emplace(filePath, GetPersistentAllocator());

			{
				GTSL::FileQuery shaderGroupInstanceFileQuery(GetUserResourcePath(filePath + u8"_*", u8"json"));

				while (auto instanceFileRef = shaderGroupInstanceFileQuery()) {
					GTSL::File shaderGroupInstanceFile(GetUserResourcePath(instanceFileRef.Get()), GTSL::File::READ, false);
					GTSL::Buffer shaderGroupInstanceBuffer(shaderGroupInstanceFile, GetTransientAllocator());
					
					auto shaderGroupInstanceJson = GTSL::JSON(GTSL::StringView(shaderGroupInstanceBuffer), GetPersistentAllocator());

					auto instanceName = instanceFileRef.Get();
					LTrimFirst(instanceName, u8'_'); 	RTrimLast(instanceName, u8'.');

					auto& instance = shaderGroupDataSerialize.Instances.EmplaceBack();
					instance.Name = instanceName;

					for (auto f : shaderGroupInstanceJson[u8"parameters"]) {
						auto& param = instance.Parameters.EmplaceBack();
						param.First = f[u8"name"];
						param.Second = f[u8"defaultValue"];
					}
				}
			}

			GTSL::File shaderGroupFile(GetUserResourcePath(fileData.Name));

			GTSL::Buffer shaderGroupBuffer(GetTransientAllocator()); shaderGroupFile.Read(shaderGroupBuffer);
			
			const auto& json = shaderGroupJSONsMap.Emplace(filePath, GTSL::StringView(shaderGroupBuffer), GetPersistentAllocator());

			// Do domain to permutation guide matching
			auto domain = json[u8"domain"];

			for(auto permutation : permutations) {
				for(auto d : permutation->GetSupportedDomains()) {
					if(GTSL::StringView(domain) == d) { // If this permutation guide supports this domain, add
						dependencyTree[GTSL::Hash(GTSL::StaticString<128>(permutation->InstanceName) + u8".bespg.json")].Connect(node); // Connect permutation guide to shader group
					}
				}
			}

			// Connect shader group to shaders in dependency tree
			for(auto shader : json[u8"shaders"]) {
				auto shaderName = shader[u8"name"];

				node.Connect(dependencyTree[GTSL::Hash(GTSL::StaticString<128>(shaderName) + u8".besh.json")]);
				node.Connect(dependencyTree[GTSL::Hash(GTSL::StaticString<128>(shaderName) + u8".besh.txt")]);
			}

			//TODO: connect shader group instances and shaders to those so when shader instances are modified shaders can be updated

			processShaderGroup(json, pipeline, commonPermutation, &shaderGroupDataSerialize, shaderMap);
		}
	};

	// Handles serialization and does compilation
	auto processTree = [&](const Graph<IFile>& processed, const Graph<IFile>& node, GTSL::StringView match, State state, auto&& self) -> void {
		if(state == State::NONE) { return; }

		auto& interestData = processed.GetData(); auto& parentData = node.GetData();

		if(GTSL::IsIn(parentData.Name, match) and state == State::MODIFIED) { // If current node is of the needed type
			auto filePath = parentData.Name; RTrimFirst(filePath, u8'.');

			auto& sgd = shaderGroupMap[filePath]; // TODO: don't remake shader group, only do if first time compiling

			if(GTSL::IsIn(interestData.Name, u8"besh.txt")) { // If interested node is shader
				auto shaderName = GTSL::StaticString<128>(interestData.Name);
				RTrimFirst(shaderName, u8'.');
				BE_LOG_MESSAGE(u8"Recompiling ", interestData.Name, u8" shader.");

				makeShaderGroup(shaderGroupJSONsMap[filePath], pipeline, commonPermutation, &sgd, shaderMap); // TODO: watch out for shader scope handles, when doing hot reloading as they will still be in the shader group data serialize

				auto shaderJSON = GTSL::Find(sgd.ShaderJSONs, [&shaderName](const GTSL::JSON<BE::TAR>& json){ return json[u8"name"] == shaderName; });

				auto shaderBinaryBuffer = compileShader(*shaderJSON.Get(), pipeline, sgd.SSS[0][shaderJSON.Get() - sgd.ShaderJSONs.begin()]);

				if(shaderBinaryBuffer.GetLength()) { // If succesfully compiled shader
					serializeShader(*shaderJSON.Get(), shaderBinaryBuffer.GetRange());
					UpdateFileHashCache(interestData.Pointer, changeCache, interestData.FileHash); // Update cached hash of file only if shader was actually updated
				} else { // Failed to compile shader
					BE_LOG_ERROR(u8"Failed to compile shader: ", shaderName, u8".");
				}

				return;
			}
		}

		if(GTSL::IsIn(interestData.Name, u8"bespg")) {
		}

		if(GTSL::IsIn(interestData.Name, u8"besh") and state == State::MODIFIED) {
			// Look for first shader group upstream and recompile the shader under that context, as shaders "cannot" be compiled stand-alone
			//BE_LOG_WARNING(u8"Parents: ")
			//for(auto& e : node.GetParents()) {
			//	BE_LOG_WARNING(e.GetData().Name);
			//}

			for(auto& e : node.GetParents()) {
				self(processed, e, u8"besg", state, self);
			}
		}

		if(GTSL::IsIn(interestData.Name, u8"besg")) {
			auto filePath = interestData.Name; RTrimFirst(filePath, u8'.');

			auto& sgd = shaderGroupMap[filePath];

			makeShaderGroup(shaderGroupJSONsMap[filePath], pipeline, commonPermutation, &sgd, shaderMap); // TODO: watch out for shader scope handles, when doing hot reloading as they will still be in the shader group data serialize

			if(interestData.Name == parentData.Name) {
				serializeShaderGroup(sgd);
			
				for(const auto& p : sgd.SSS) {
					BE_ASSERT(p.GetLength() == sgd.ShaderJSONs.GetLength(), u8"");

					for(uint32 i = 0; i < p; ++i) {
						auto& s = p[i];
						const auto& shaderJSON = sgd.ShaderJSONs[i];

						auto shaderBinaryBuffer = compileShader(shaderJSON, pipeline, s);
						serializeShader(shaderJSON, shaderBinaryBuffer.GetRange());
					}
				}
			}

			return;
		}
	};

	if(changedFiles) { // If there are any changed files, load all info
		for(auto& e : fileTree) { // Visit file tree to build all state
			populateTree(dependencyTree[GTSL::Hash(e.GetData().Name)], e.GetData().State);
		}

		for(auto& e : changedFiles) { // Visit new files and add them to the tree
			if(fileTree.Find(GTSL::Hash(e.Name)) and e.State != State::DELETED) { continue; }
			populateTree(dependencyTree[GTSL::Hash(e.Name)], e.State);
		}
	}

	{
		uint32 offset = 0;

		while(offset != shaderInfoTableFile.GetSize()) {
			offset = ReadIndexEntry(shaderInfoTableFile, offset, [&](const uint64 offs, const GTSL::StringView name) {
				shaderInfoOffsets.Emplace(GTSL::Hash(name), offs);
				shaderInfoPointers.Emplace(GTSL::Hash(name), offset);
			});		
		}
	}

	{
		uint32 offset = 0;

		while(offset != shadersTableFile.GetSize()) {
			offset = ReadIndexEntry(shadersTableFile, offset, [&](const uint64 offs, const GTSL::StringView name) {
				shaderOffsets.Emplace(GTSL::Hash(name), offs);
				shadersPointer.Emplace(GTSL::Hash(name), offset);
			});
		}
	}

	// After the whole tree is populated and info pointers have been loaded
	// Visit the tree once more now compiling everything based on the data that was generated earlier

	for(auto& node : dependencyTree) {
		processTree(node, node, u8"xxxxxxxxxxx", node.GetData().State, processTree);
	}

	auto printDependencyTree = [&](uint64 parent_file_name_hash, const Graph<IFile>& node, auto&& self) -> void {
		auto& nodeData = node.GetData();

		UpdateParentFileNameCache(nodeData.Pointer, changeCache, parent_file_name_hash);

		//BE_LOG_SUCCESS(u8"Parent: ", parent_file_name_hash, u8"Node: ", node.GetData().Name);
		for(auto& e : node.GetChildren()) {
			//BE_LOG_SUCCESS(u8"Child: ", e.GetData().Name);
			self(GTSL::Hash(nodeData.Name), e, self);
		}
	};

	if(changedFiles) {
		printDependencyTree(0ull, dependencyTree[GTSL::Hash(u8"CommonPermutation.bespg.json")], printDependencyTree);
	}

	for (auto& e : changedFiles) {
		if(e.State != State::DELETED) { continue; }
		// TODO: delete
	}

	{
		uint32 offset = 0;

		while(offset != shaderGroupsTableFile.GetSize()) {
			offset = ReadIndexEntry(shaderGroupsTableFile, offset, [&](const uint64 offset, const GTSL::StringView name) { shaderGroupInfoOffsets.Emplace(Id(name), offset); });
		}
	}
}

inline void ParseStructJSONAndDeclareStruct(const GTSL::JSON<BE::PAR>& json, GPipeline& pipeline, GPipeline::ElementHandle scope) {
	GTSL::StaticVector<StructElement, 8> elements;

	for (auto m : json[u8"members"]) {
		elements.EmplaceBack(m[u8"type"], m[u8"name"]);
	}

	pipeline.DeclareStruct(scope, json[u8"name"], elements);
}

inline void ShaderResourceManager::processShaderGroup(const GTSL::JSON<BE::PAR>& json, GPipeline& pipeline,
	PermutationManager* root_permutation, ShaderGroupDataSerialize* shader_group_data_serialize, const ShaderMap& shader_map) {
	auto shaderGroupScope = pipeline.DeclareScope(GPipeline::GLOBAL_SCOPE, json[u8"name"]);

	pipeline.DeclareConstant(shaderGroupScope, { u8"uint32", u8"DEBUG", u8"0" });

	for (auto s : json[u8"structs"]) {
		ParseStructJSONAndDeclareStruct(s, pipeline, shaderGroupScope);
	}

	for(auto scope : json[u8"scopes"]) {
		auto scopeHandle = pipeline.DeclareScope(shaderGroupScope, scope[u8"name"]);

		for(auto e : scope[u8"elements"]) {
			pipeline.DeclareVariable(scopeHandle, { e[u8"type"], e[u8"name"] });
		}
	}

	if(auto dataLayers = json[u8"dataLayers"]) {
		auto pcb = pipeline.DeclareScope(shaderGroupScope, u8"pushConstantBlock");

		for(auto dataLayer : dataLayers) {
			pipeline.DeclareVariable(pcb, { dataLayer[u8"type"], dataLayer[u8"name"] });
		}
	}

	for (auto f : json[u8"functions"]) {
		GTSL::StaticVector<StructElement, 16> elements;
		for (auto p : f[u8"parameters"]) { elements.EmplaceBack(p[u8"type"], p[u8"name"]); }
		pipeline.DeclareFunction(shaderGroupScope, f[u8"type"], f[u8"name"], elements, f[u8"code"]);
	}

	shader_group_data_serialize->Name = json[u8"name"];
	
	for (auto e : json[u8"tags"]) {
		shader_group_data_serialize->Tags.EmplaceBack(e[u8"name"].GetStringView(), e[u8"value"].GetStringView());
	}

	{
		GPipeline::ElementHandle shaderParametersDataHandle;
		GTSL::StaticVector<StructElement, 16> structElements;

		for (auto p : json[u8"parameters"]) {
			if (auto def = p[u8"defaultValue"]) {
				shader_group_data_serialize->Parameters.EmplaceBack(p[u8"type"], p[u8"name"], def);
				structElements.EmplaceBack(p[u8"type"], p[u8"name"], def);
			} else {
				shader_group_data_serialize->Parameters.EmplaceBack(p[u8"type"], p[u8"name"], u8"");
				structElements.EmplaceBack(p[u8"type"], p[u8"name"], u8"");
			}
		}

		shaderParametersDataHandle = pipeline.DeclareStruct(shaderGroupScope, u8"ShaderParametersData", structElements);
	}

	shader_group_data_serialize->VertexElements.EmplaceBack().EmplaceBack(u8"vec3f", u8"POSITION");
	shader_group_data_serialize->VertexElements.EmplaceBack().EmplaceBack(u8"vec3f", u8"NORMAL");
	shader_group_data_serialize->VertexElements.EmplaceBack().EmplaceBack(u8"vec3f", u8"TANGENT");
	shader_group_data_serialize->VertexElements.EmplaceBack().EmplaceBack(u8"vec3f", u8"BITANGENT");
	shader_group_data_serialize->VertexElements.EmplaceBack().EmplaceBack(u8"vec2f", u8"TEXTURE_COORDINATES");

	if(!shader_group_data_serialize->Instances) {
		auto& basicInstance = shader_group_data_serialize->Instances.EmplaceBack();
		basicInstance.Name = json[u8"name"];
	}

	bool rayTrace = false; ShaderGroupInfo::RayTraceData ray_trace_data;

	auto shaderGroupDomain = json[u8"domain"];

	shader_group_data_serialize->RayTrace = rayTrace;
	shader_group_data_serialize->RayTraceData = ray_trace_data;
}

inline void ShaderResourceManager::makeShaderGroup(const GTSL::JSON<BE::PAR>& json, GPipeline& pipeline, PermutationManager* root_permutation, ShaderGroupDataSerialize* shader_group_data_serialize, const ShaderMap& shader_map) {
	auto shaderGroupScope = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, json[u8"name"]);
	auto& scopesPerPermutations = shader_group_data_serialize->SSS;

	const auto shaderGroupDomain = json[u8"domain"];

	auto evaluateForPermutation = [&](PermutationManager* permutation_manager, GTSL::StaticVector<GPipeline::ElementHandle, 16> permutationScopes, auto&& self) -> void {
		auto permutationScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, permutation_manager->InstanceName);
		permutationScopes.EmplaceBack(permutationScopeHandle); //Insert permutation handle

		// If permutation handles this shader group's domains, evalute shader group for permutation, else check if children can handle this domain and then return
		if (!Contains(permutation_manager->GetSupportedDomains(), GTSL::StringView(shaderGroupDomain))) {
			for(auto e : permutation_manager->Children) {
				self(e, permutationScopes, self);
			}

			return;
		}

		auto& xxx = scopesPerPermutations.EmplaceBack();

		permutationScopes.EmplaceBack(shaderGroupScope);

		auto auxScopes = permutationScopes;

		for (auto s : json[u8"shaders"]) {
			permutationScopes = auxScopes; //reset scopes for every shader built

			if(!shader_map.Find(s[u8"name"])) { BE_LOG_WARNING(u8"Could not find any shader under name: ", GTSL::StringView(s[u8"name"])); continue; }

			auto& shaderEntry = shader_map[s[u8"name"]];
			auto& shaderJson = GTSL::Get<0>(shaderEntry);

			GTSL::StaticString<64> executionString;

			shaderJson[u8"execution"](executionString);

			if (GTSL::StringView(shaderGroupDomain) == GTSL::StringView(u8"Screen")) {
				executionString = GTSL::ShortString<64>(u8"windowExtent");
			}

			GPipeline::ElementHandle shaderScope = pipeline.DeclareShader(shaderGroupScope, shaderJson[u8"name"]);

			auto pcd = pipeline.DeclareScope(shaderScope, u8"pushConstantBlock");

			pipeline.DeclareVariable(pcd, { u8"ShaderParametersData*", u8"shaderParameters" });

			for(auto sharedVariableJSON : shaderJson[u8"sharedVariables"]) {
				pipeline.DeclareShared(shaderScope, { sharedVariableJSON[u8"type"], sharedVariableJSON[u8"name"] });
			}

			if (shaderJson[u8"class"].GetStringView() == GTSL::StringView(u8"COMPUTE") or shaderJson[u8"class"].GetStringView() == GTSL::StringView(u8"RAY_GEN")) {
				GTSL::StaticString<64> x, y, z;

				if(auto res = shaderJson[u8"localSize"]) {
					GTSL::ToString(x, res[0].GetUint()); GTSL::ToString(y, res[1].GetUint()); GTSL::ToString(z, res[2].GetUint());
				} else {
					x = u8"1"; y = u8"1"; z = u8"1";
				}

				pipeline.DeclareVariable(shaderScope, { u8"uint16", u8"group_size_x", x });
				pipeline.DeclareVariable(shaderScope, { u8"uint16", u8"group_size_y", y });
				pipeline.DeclareVariable(shaderScope, { u8"uint16", u8"group_size_z", z });
			}

			uint32 i = 0, j = 0; bool found = false;

			for(; i < permutation_manager->JSON[u8"options"]; ++i) {
				if(permutation_manager->JSON[u8"options"][i][u8"domain"] == GTSL::StringView(shaderGroupDomain)) {
					for(; j < permutation_manager->JSON[u8"options"][i][u8"subsets"]; ++j) {
						auto shaderClass = GTSL::StaticString<64>(shaderJson[u8"class"]);

						for(uint32 k = 0; k < permutation_manager->JSON[u8"options"][i][u8"subsets"][j][u8"sourceClasses"]; ++k) {
							auto s = GTSL::StaticString<64>(permutation_manager->JSON[u8"options"][i][u8"subsets"][j][u8"sourceClasses"][k]);
							if(s == shaderClass) {
								found = true;
								break;
							}							
						}

						if(found) break;
					}

					if(found) break;
				}
			}

			if(!found or !(i < permutation_manager->a) or !(j < permutation_manager->a[i])) {
				BE_LOG_WARNING(u8"Could not generate shader: ", GTSL::StringView(shaderJson[u8"name"]), u8", for shader group: ", GTSL::StringView(json[u8"name"]), u8", because a shader template for permutation: ", permutation_manager->InstanceName, (i < permutation_manager->a) ? GTSL::StaticString<128>(u8", option: ") + GTSL::StringView(permutation_manager->JSON[u8"options"][i][u8"name"]) : GTSL::StaticString<128>(), (i < permutation_manager->a && j < permutation_manager->a[i]) ? GTSL::StaticString<128>(u8", subset: ") + GTSL::StringView(permutation_manager->JSON[u8"options"][i][u8"subsets"][j][u8"name"]) : GTSL::StaticString<128>(), u8", was not available.");
				return;
			}

			auto& shaderFileSourceCode = shaderEntry.rest.element;

			GTSL::Vector<ShaderNode, BE::TAR> tokens(GetTransientAllocator());
			tokenizeCode(shaderFileSourceCode, tokens, GetTransientAllocator());

			for(uint32 t = 0u; t < tokens; ++t) {
				if(tokens[t].Name != u8"function") { continue; }

				++t; // Skip "function" token

				auto returnType = tokens[t].Name;
				++t;
				auto functionName = tokens[t].Name;
				++t;

				++t; // Skip open parenthesis

				GTSL::StaticVector<StructElement, 16> parameters;

				while(true) {
					if(tokens[t].Name == u8")") { ++t; break; }

					auto parameterReturnType = tokens[t].Name;
					++t;

					if(parameterReturnType == u8"inout" or parameterReturnType == u8"in" or parameterReturnType == u8"out") {
						parameterReturnType &= tokens[t].Name;
						++t;
					}

					auto parameterName = tokens[t].Name;
					++t;
					parameters.EmplaceBack(parameterReturnType, parameterName);
					if(tokens[t].ValueType == ShaderNode::Type::COMMA) { ++t; }
				}

				++t; // Skip open brace

				const auto shaderGuide = GTSL::StringView(permutation_manager->a[i][j]);

				auto b = shaderGuide.begin();

				while(b != shaderGuide.end() && *b != u8'@') {
					++b;
				}

				auto functionHandle = pipeline.DeclareFunction(shaderScope, returnType, functionName, parameters);

				if(functionName == u8"main") {
					pipeline.AddCodeToFunction(functionHandle, { shaderGuide.begin(), b });
				}

				uint32 startFunctionCode = t;

				uint32 openCloseCounter = 1u; // Must be one because we skipped over open brace
				while(true) {
					if(tokens[t].Name == u8"}") {
						--openCloseCounter;
						if(!openCloseCounter) {
							break;
						}
					}
					if(tokens[t].Name == u8"{") { ++openCloseCounter; }
					++t;
				}

				pipeline.AddCodeToFunction(functionHandle, {t - startFunctionCode, tokens.begin() + startFunctionCode});

				if(functionName == u8"main") {
					pipeline.AddCodeToFunction(functionHandle, { ++b, shaderGuide.end() });
				}
			}

			{
				// FIX
				{
					auto commonPermutationScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"CommonPermutation");

					Class shaderClass = ShaderClassFromString(shaderJson[u8"class"]);

					GPipeline::ElementHandle subScopeHandle;

					switch (shaderClass) {
						case Class::VERTEX: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"VertexShader"); break;
						case Class::SURFACE: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"FragmentShader"); break;
						case Class::COMPUTE: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"ComputeShader");
						permutationScopes.EmplaceBack(pipeline.GetElementHandle(commonPermutationScopeHandle, u8"ComputeRenderPass"));
						break;
						case Class::RENDER_PASS: break;
						case Class::CLOSEST_HIT: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"ClosestHitShader"); break;
						case Class::RAY_GEN: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"RayGenShader"); break;
						case Class::MISS: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"MissShader"); break;
					}

					permutationScopes.EmplaceBack(subScopeHandle);
				}
				// FIX

				permutationScopes.EmplaceBack(shaderScope);

				GTSL::String shaderJsonString(GetTransientAllocator());

				auto serializer = GTSL::MakeSerializer(shaderJsonString);

				GTSL::Insert(serializer, shaderJsonString, u8"name", shaderJson[u8"name"]);
				GTSL::Insert(serializer, shaderJsonString, u8"binarySize", 0ull);
				GTSL::Insert(serializer, shaderJsonString, u8"binaryHash", 0ull);
				GTSL::Insert(serializer, shaderJsonString, u8"class", shaderJson[u8"class"]);
				GTSL::Insert(serializer, shaderJsonString, u8"targetSemantics", permutation_manager->JSON[u8"options"][i][u8"subsets"][j][u8"targetSemantics"]);

				GTSL::StartArray(serializer, shaderJsonString, u8"tags");

				for(auto e : permutation_manager->JSON[u8"tags"]) {
					GTSL::StartObject(serializer, shaderJsonString);

					GTSL::Insert(serializer, shaderJsonString, u8"name", e[u8"name"]);
					GTSL::Insert(serializer, shaderJsonString, u8"value", e[u8"value"]);

					GTSL::EndObject(serializer, shaderJsonString);
				}

				for(auto e : json[u8"tags"]) {
					GTSL::StartObject(serializer, shaderJsonString);

					GTSL::Insert(serializer, shaderJsonString, u8"name", e[u8"name"]);
					GTSL::Insert(serializer, shaderJsonString, u8"value", e[u8"value"]);

					GTSL::EndObject(serializer, shaderJsonString);
				}

				for(auto e : shaderJson[u8"tags"]) {
					GTSL::StartObject(serializer, shaderJsonString);

					GTSL::Insert(serializer, shaderJsonString, u8"name", e[u8"name"]);
					GTSL::Insert(serializer, shaderJsonString, u8"value", e[u8"value"]);

					GTSL::EndObject(serializer, shaderJsonString);
				}

				GTSL::EndArray(serializer, shaderJsonString);

				GTSL::EndSerializer(shaderJsonString, serializer);
				
				shader_group_data_serialize->ShaderJSONs.EmplaceBack(GTSL::JSON(shaderJsonString, GetTransientAllocator()));

				shader_group_data_serialize->Shaders.EmplaceBack(GTSL::Hash(shaderJson[u8"name"]));

				xxx.EmplaceBack(permutationScopes);
			}
		}
	};

	evaluateForPermutation(root_permutation, {GPipeline::GLOBAL_SCOPE}, evaluateForPermutation);
}

inline void ShaderResourceManager::serializeShaderGroup(const ShaderGroupDataSerialize& shader_group_data_serialize) {
	for(auto& e :shader_group_data_serialize.Instances) {
		WriteIndexEntry(shaderGroupsTableFile, ~0ULL, shaderGroupInfosFile.GetSize(), e.Name); // write offset to shader group for each instance name
	}

	{
		shaderGroupInfosFile << shader_group_data_serialize.Name;

		shaderGroupInfosFile << shader_group_data_serialize.Shaders.GetLength();
		for (auto& e : shader_group_data_serialize.Shaders) { shaderGroupInfosFile << e; }

		shaderGroupInfosFile << shader_group_data_serialize.Parameters.GetLength();
		for (auto& p : shader_group_data_serialize.Parameters) {
			shaderGroupInfosFile << p.Type << p.Name << p.Value;
		}

		shaderGroupInfosFile << shader_group_data_serialize.Tags.GetLength();
		for (auto& p : shader_group_data_serialize.Tags) {
			shaderGroupInfosFile << p.First << p.Second;
		}

		shaderGroupInfosFile << shader_group_data_serialize.Instances.GetLength();
		for (auto& i : shader_group_data_serialize.Instances) {

			shaderGroupInfosFile << i.Name;

			shaderGroupInfosFile << i.Parameters.GetLength();
			for (auto& p : i.Parameters) {
				shaderGroupInfosFile << p.First << p.Second;
			}
		}


		shaderGroupInfosFile << shader_group_data_serialize.VertexElements.GetLength();
		for (auto& e : shader_group_data_serialize.VertexElements) {
			shaderGroupInfosFile << e.GetLength();
			for (auto& ve : e) {
				shaderGroupInfosFile << ve.Type << ve.Name;
			}
		}

		shaderGroupInfosFile << shader_group_data_serialize.RayTrace;

		if (shader_group_data_serialize.RayTrace) {
			const auto& rayTraceData = shader_group_data_serialize.RayTraceData;
			shaderGroupInfosFile << rayTraceData.Payload.Type << rayTraceData.Payload.Name << rayTraceData.Payload.DefaultValue;

			for (uint32 i = 0; i < 4; ++i) {
				shaderGroupInfosFile << rayTraceData.Groups[i].ShadersPerGroup.GetLength();

				for (uint32 j = 0; j < rayTraceData.Groups[i].ShadersPerGroup.GetLength(); ++j) {
					shaderGroupInfosFile << rayTraceData.Groups[i].ShadersPerGroup[j];
				}
			}
		}
	}
}

inline GTSL::Buffer<BE::TransientAllocatorReference> ShaderResourceManager::compileShader(
	const GTSL::JSON<BE::TAR>& json, const GPipeline& pipeline,
	const GTSL::Range<const GPipeline::ElementHandle*> source_scopes) {
	GTSL::StaticVector<GTSL::StringView, 16> scopesStrings;

	GTSL::StaticVector<GPipeline::ElementHandle, 16> scopes(source_scopes);	

	// Make shader name by appending all the names of the scopes that comprise, which allows to easily identify the permutation
	for (auto& e : scopes) { if (auto& n = pipeline.GetElement(e).Name) { scopesStrings.EmplaceBack(n); } }

	GTSL::StaticString<64> shaderName = GTSL::StringView(json[u8"name"]);
	GTSL::StaticString<512> qualifiedShaderName; qualifiedShaderName += GTSL::Join{ scopesStrings, u8"." };

	const auto targetSemantics = ShaderTypeFromString(json[u8"targetSemantics"]);

	auto shaderCode = GenerateShader(pipeline, scopes, targetSemantics, GetTransientAllocator());
	if (!shaderCode) { BE_LOG_WARNING(shaderCode.Get().Second); }

	auto [compilationSuccessCode, compilationErrors, shaderBinaryBuffer] = compiler_.Compile(shaderCode.Get().First, shaderName, targetSemantics, GAL::ShaderLanguage::GLSL, true, GetTransientAllocator());

	if (!compilationSuccessCode) {
		GTSL::Console::Print(shaderCode.Get().First);
		BE_LOG_ERROR(compilationErrors);
	}

	return GTSL::MoveRef(shaderBinaryBuffer);
}

inline void ShaderResourceManager::serializeShader(const GTSL::JSON<BE::TAR>& json, GTSL::Range<const byte*> shader_binary_buffer) {
	auto a = shaderInfoPointers.TryEmplace(GTSL::Hash(json[u8"name"]), ~0ULL);
	auto b = shadersPointer.TryEmplace(GTSL::Hash(json[u8"name"]), ~0ULL);

	auto nameHash = GTSL::Hash(json[u8"name"]);

	auto x = shaderInfosFile.GetSize();
	auto y = shaderPackageFile.GetSize();

	a.Get() = WriteIndexEntry(shaderInfoTableFile, a.Get(), x, json[u8"name"]);
	b.Get() = WriteIndexEntry(shadersTableFile, b.Get(), y, json[u8"name"]);

	if(shaderInfoOffsets.Find(nameHash)) {
		shaderInfoOffsets[nameHash] = x;
	} else {
		shaderInfoOffsets.Emplace(nameHash, x);
	}

	if(shaderOffsets.Find(nameHash)) {
		shaderOffsets.At(nameHash) = y;
	} else {
		shaderOffsets.Emplace(nameHash, y);
	}

	GTSL::String string(GetTransientAllocator());

	GTSL::JSONSerializer serializer = GTSL::MakeSerializer(string);

	GTSL::Insert(serializer, string, u8"name", GTSL::StringView(json[u8"name"]));
	GTSL::Insert(serializer, string, u8"binarySize", shader_binary_buffer.Bytes());
	const auto shaderBinaryHash = quickhash64(shader_binary_buffer);
	GTSL::Insert(serializer, string, u8"binaryHash", shaderBinaryHash);
	GTSL::Insert(serializer, string, u8"targetSemantics", json[u8"targetSemantics"]);

	GTSL::StartArray(serializer, string, u8"tags");

	for (auto t : json[u8"tags"]) {
		GTSL::StartObject(serializer, string);
		GTSL::Insert(serializer, string, u8"name", t[u8"name"]);
		GTSL::Insert(serializer, string, u8"value", t[u8"value"]);
		GTSL::EndObject(serializer, string);
	}

	GTSL::EndArray(serializer, string);

	//pipeline.MakeJSON(string, scopes); //MAKE JSON

	GTSL::EndSerializer(string, serializer);

	shaderInfosFile.SetPointer(x);
	shaderInfosFile << string;
	shaderPackageFile.SetPointer(y);
	shaderPackageFile.Write(shader_binary_buffer);
}