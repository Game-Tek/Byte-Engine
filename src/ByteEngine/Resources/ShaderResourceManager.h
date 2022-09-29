#pragma once

#include <GAL/Pipelines.h>
#include <GAL/RenderCore.h>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/String.hpp>
#include <GTSL/File.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/Serialize.hpp>
#include <GTSL/Filesystem.h>
#include <GAL/Serialize.hpp>

#include "ResourceManager.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Render/ShaderGenerator.h"

#include "PermutationManager.hpp"
#include "CommonPermutation.hpp"
#include "ForwardPermutation.hpp"
#include "UIPermutation.hpp"
#include "RayTracePermutation.hpp"

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
		GTSL::Vector<uint32, BE::PAR> Shaders;
		GTSL::Vector<PermutationManager::ShaderTag, BE::PAR> Tags;
		GTSL::StaticVector<GTSL::StaticVector<StructElement, 8>, 8> VertexElements;
	};
	
	struct ShaderGroupDataSerialize : ShaderGroupData, Object {
		ShaderGroupDataSerialize(const BE::PAR& allocator) : ShaderGroupData(allocator) {}
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
	GTSL::HashMap<uint64, uint64, BE::PersistentAllocatorReference> shaderInfoOffsets, shaderOffsets;

	mutable GTSL::ReadWriteMutex mutex;

	GAL::ShaderCompiler compiler_;

	using ShaderMap = GTSL::HashMap<Id, GTSL::Tuple<GTSL::JSON<BE::PAR>, GTSL::StaticString<2048>>, BE::TAR>;

	void makeShaderGroup(const GTSL::JSON<BE::PAR>& json, GPipeline& pipeline, PermutationManager* root_permutation, ShaderGroupDataSerialize*, const ShaderMap& shader_map);

	template<typename... ARGS>
	void loadShaderGroup(TaskInfo taskInfo, Id shaderGroupName, TaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS... args) { //TODO: check why can't use ARGS&&
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
					auto& tag = shader.Tags.EmplaceBack();
					shaderInfosFile >> tag.First >> tag.Second;
				}

				shaderInfosFile >> shader.DebugData;
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
			shaderPackageFile.SetPointer(shaderOffsets[s.Hash]);
			shaderPackageFile.Read(s.Size, buffer.begin() + offset);
			offset += s.Size;
		}

		taskInfo.ApplicationManager->EnqueueTask(dynamicTaskHandle, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
	}

	using Result = GTSL::StaticVector<PermutationManager::ShaderPermutation, 8>;

	GTSL::File shaderGroupsTableFile, shaderInfoTableFile, shadersTableFile;
	GTSL::KeyMap<ShaderHash, BE::PAR> loadedShaders;
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

inline GAL::ShaderType ShaderTypeFromString(GTSL::StringView string) {
	switch (GTSL::Hash(string)) {
		case GTSL::Hash(u8"VERTEX"): return GAL::ShaderType::VERTEX;
		case GTSL::Hash(u8"FRAGMENT"): return GAL::ShaderType::FRAGMENT;
		case GTSL::Hash(u8"COMPUTE"): return GAL::ShaderType::COMPUTE;
		case GTSL::Hash(u8"RAY_GEN"): return GAL::ShaderType::RAY_GEN;
		case GTSL::Hash(u8"CLOSEST_HIT"): return GAL::ShaderType::CLOSEST_HIT;
		case GTSL::Hash(u8"ANY_HIT"): return GAL::ShaderType::ANY_HIT;
		case GTSL::Hash(u8"MISS"): return GAL::ShaderType::MISS;
	}
}

inline Class ShaderClassFromString(GTSL::StringView string) {
	switch (GTSL::Hash(string)) {
		case GTSL::Hash(u8"VERTEX"): return Class::VERTEX;
		case GTSL::Hash(u8"SURFACE"): return Class::SURFACE;
		case GTSL::Hash(u8"COMPUTE"): return Class::COMPUTE;
		case GTSL::Hash(u8"RAY_GEN"): return Class::RAY_GEN;
		case GTSL::Hash(u8"CLOSEST_HIT"): return Class::CLOSEST_HIT;
		case GTSL::Hash(u8"MISS"): return Class::MISS;
	}
}

inline ShaderResourceManager::ShaderResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"ShaderResourceManager"), shaderGroupInfoOffsets(8, GetPersistentAllocator()), shaderInfoOffsets(8, GetPersistentAllocator()), shaderOffsets(8, GetPersistentAllocator()), loadedShaders(16, GetPersistentAllocator()) {
	shaderPackageFile.Open(GetResourcePath(u8"Shaders", u8"bepkg"), GTSL::File::READ | GTSL::File::WRITE, true);

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

	GTSL::HashMap<GTSL::StringView, PermutationManager*, BE::TAR> permutations(8, GetTransientAllocator());

	GTSL::SmartPointer<CommonPermutation, BE::TAR> commonPermutation(GetTransientAllocator(), u8"CommonPermutation");

	permutations.Emplace(u8"CommonPermutation", static_cast<PermutationManager*>(commonPermutation.GetData()));

	GPipeline pipeline;

	{ //configure permutations
		permutations.Emplace(u8"ForwardRenderingPermutation", commonPermutation->CreateChild<ForwardRenderPassPermutation>(&pipeline, u8"ForwardRenderingPermutation"));
		permutations.Emplace(u8"RayTracePermutation", commonPermutation->CreateChild<RayTracePermutation>(&pipeline, u8"RayTracePermutation"));
		permutations.Emplace(u8"UIPermutation", commonPermutation->CreateChild<UIPermutation>(&pipeline, u8"UIPermutation"));
	}

	PermutationManager::InitializePermutations(commonPermutation, &pipeline);

	{
		changeCache.Open(GetResourcePath(u8"ShaderResourceManagerCache.bin"), GTSL::File::READ | GTSL::File::WRITE, true);

		GTSL::Buffer<GTSL::StaticAllocator<4096>> cacheBuffer;

		changeCache.Read(cacheBuffer);

		uint32 cacheEntryCount = cacheBuffer.GetLength() / 8;
		uint64* cacheEntries = reinterpret_cast<uint64*>(cacheBuffer.begin());

		GTSL::KeyMap<uint64, BE::TAR> entriesMap(32, GetTransientAllocator());

		for(uint32 i = 0; i < cacheEntryCount; ++i) {
			entriesMap.Emplace(cacheEntries[i]);
		}

		GTSL::FileQuery shaderGroupFileQuery(GetUserResourcePath(u8"*ShaderGroup", u8"json"));

		while (auto fileRef = shaderGroupFileQuery()) {
			if(!entriesMap.Find(shaderGroupFileQuery.GetFileHash())) {
				changeCache << shaderGroupFileQuery.GetFileHash();
				created = true;
			}
		}

		GTSL::FileQuery shaderQuery(GetUserResourcePath(u8"*Shader", u8"json"));

		while (auto fileRef = shaderQuery()) {
			if(!entriesMap.Find(shaderQuery.GetFileHash())) {
				changeCache << shaderQuery.GetFileHash();
				created = true;
			}
		}
	}

	if (!(shaderPackageFile.GetSize() && shaderGroupsTableFile.GetSize() && shaderInfoTableFile.GetSize() && shadersTableFile.GetSize() && shaderInfosFile.GetSize() && shaderGroupInfosFile.GetSize()) or created) {
		shaderPackageFile.Resize(0);
		shaderGroupsTableFile.Resize(0);
		shaderInfoTableFile.Resize(0);
		shadersTableFile.Resize(0);
		shaderInfosFile.Resize(0);
		shaderGroupInfosFile.Resize(0);
		created = true;
	}

	if (created) {
		GTSL::Vector<GTSL::StaticString<512>, BE::TAR> paths(8, GetTransientAllocator());

		{
			GTSL::FileQuery renderingGuideFileQuery(GetUserResourcePath(u8"*.bespg.json"));

			while(auto fileRef = renderingGuideFileQuery()) {
				GTSL::File file(GetUserResourcePath(fileRef.Get()));

				GTSL::StaticBuffer<2048> fileBuffer(file);

				auto json = GTSL::JSON(GTSL::StringView(fileBuffer), GetPersistentAllocator());

				auto permutation = permutations[GTSL::StringView(json[u8"name"])];

				pipeline.TryDeclareScope(GPipeline::GLOBAL_SCOPE, GTSL::StringView(json[u8"name"]));

				for(auto option : json[u8"options"]) {
					auto& a = permutation->a.EmplaceBack(GetPersistentAllocator());

					permutation->AddSupportedDomain(option[u8"domain"]);

					for(auto subset : option[u8"subsets"]) {
						GTSL::StaticString<128> subsetPath;
						subsetPath += GTSL::Join { { GTSL::StringView(json[u8"name"]), GTSL::StringView(option[u8"name"]), GTSL::StringView(subset[u8"name"]) }, u8"." };
						GTSL::File guideSubsetShaderTemplateFile(GetUserResourcePath(subsetPath, u8"txt"));

						if(guideSubsetShaderTemplateFile) {
							a.EmplaceBack(guideSubsetShaderTemplateFile, GetPersistentAllocator());
						} else {
							BE_LOG_WARNING(u8"Didn't find any shader subset template for subset: ", GTSL::StringView(subset[u8"name"]), u8".");
						}
					}
				}

				permutation->JSON = GTSL::MoveRef(json);
			}
		}

		ShaderMap shaderMap(32, GetTransientAllocator());

		{
			GTSL::FileQuery shaderFilesQuery(GetUserResourcePath(u8"*.besh.json"));

			while(auto e = shaderFilesQuery()) {
				GTSL::File jsonShaderFile(GetUserResourcePath(e.Get()));
				GTSL::StaticBuffer<2048> jsonShaderFileBuffer(jsonShaderFile);
				
				auto json = GTSL::JSON(GTSL::StringView(jsonShaderFileBuffer), GetPersistentAllocator());

				auto& shaderEntry = shaderMap.Emplace(Id(json[u8"name"]), GetPersistentAllocator(), GTSL::StringView());

				if(auto code = json[u8"code"]) {
					shaderEntry.rest.element = GTSL::StringView(code);
				} else {
					GTSL::File shaderFile(GetUserResourcePath(json[u8"name"], u8"txt"));

					if(shaderFile) {
						GTSL::StaticBuffer<4096> shaderFileBuffer(shaderFile);
						shaderEntry.rest.element = GTSL::StringView(shaderFileBuffer);
					} else {
						BE_LOG_WARNING(u8"Did not find a shader file for shader: ", json[u8"name"], u8".");
					}
				}

				shaderEntry.element = GTSL::MoveRef(json);
			}			
		}

		GTSL::FileQuery shaderGroupFileQuery(GetUserResourcePath(u8"*.besg.json"));

		while (auto fileRef = shaderGroupFileQuery()) {
			GTSL::File shaderGroupFile(GetUserResourcePath(fileRef.Get()));

			ShaderGroupDataSerialize shaderGroupDataSerialize(GetPersistentAllocator());

			{
				auto filePath = fileRef.Get();
				RTrimFirst(filePath, u8'.');

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

			GTSL::Buffer shaderGroupBuffer(GetTransientAllocator()); shaderGroupFile.Read(shaderGroupBuffer);
			
			auto json = GTSL::JSON(GTSL::StringView(shaderGroupBuffer), GetPersistentAllocator());

			makeShaderGroup(json, pipeline, commonPermutation, &shaderGroupDataSerialize, shaderMap); //todo: transmit instances
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

inline void ParseStructJSONAndDeclareStruct(const GTSL::JSON<BE::PAR>& json, GPipeline& pipeline, GPipeline::ElementHandle scope) {
	GTSL::StaticVector<StructElement, 8> elements;

	for (auto m : json[u8"members"]) {
		elements.EmplaceBack(m[u8"type"], m[u8"name"]);
	}

	pipeline.DeclareStruct(scope, json[u8"name"], elements);
}

inline void ShaderResourceManager::makeShaderGroup(const GTSL::JSON<BE::PAR>& json, GPipeline& pipeline, PermutationManager* root_permutation, ShaderGroupDataSerialize* shader_group_data_serialize, const ShaderMap& shader_map) {
	auto shaderGroupScope = pipeline.DeclareScope(GPipeline::GLOBAL_SCOPE, json[u8"name"]);

	GTSL::StaticVector<uint64, 16> shaderGroupUsedShaders;

	for (auto s : json[u8"structs"]) {
		ParseStructJSONAndDeclareStruct(s, pipeline, shaderGroupScope);
	}

	for(auto scope : json[u8"scopes"]) {
		auto scopeHandle = pipeline.DeclareScope(shaderGroupScope, scope[u8"name"]);

		for(auto e : scope[u8"elements"]) {
			pipeline.DeclareVariable(scopeHandle, { e[u8"type"], e[u8"name"] });
		}
	}

	for (auto f : json[u8"functions"]) {
		GTSL::StaticVector<StructElement, 8> elements;
		for (auto p : f[u8"params"]) { elements.EmplaceBack(p[u8"type"], p[u8"name"]); }
		pipeline.DeclareFunction(shaderGroupScope, f[u8"type"], f[u8"name"], elements, f[u8"code"]);
	}

	shader_group_data_serialize->Name = json[u8"name"];
	
	for (auto e : json[u8"tags"]) {
		shader_group_data_serialize->Tags.EmplaceBack(e[u8"name"].GetStringView(), e[u8"value"].GetStringView());
	}

	GPipeline::ElementHandle shaderParametersDataHandle;

	{
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

	bool rayTrace = true; ShaderGroupInfo::RayTraceData ray_trace_data;

	auto shaderGroupDomain = json[u8"domain"];

	auto evaluateForPermutation = [&](PermutationManager* parent, GTSL::StaticVector<GPipeline::ElementHandle, 16> scopes, auto&& self) -> void {
		auto permutationScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, parent->InstanceName);
		scopes.EmplaceBack(permutationScopeHandle); //Insert permutation handle

		if (!Contains(parent->GetSupportedDomains().GetRange(), GTSL::StringView(shaderGroupDomain))) {
			for(auto e : parent->Children) {
				self(e, scopes, self);
			}

			return;
		}

		auto auxScopes = scopes;

		for (auto s : json[u8"shaders"]) {
			scopes = auxScopes; //reset scopes for every shader built

			if(!shader_map.Find(Id(s[u8"name"]))) { BE_LOG_WARNING(u8"Could not find any shader under name: ", GTSL::StringView(s[u8"name"])); continue; }

			auto& shaderEntry = shader_map[Id(s[u8"name"])];
			auto& shaderJson = GTSL::Get<0>(shaderEntry);

			GTSL::StaticVector<PermutationManager::ShaderTag, 16> tags;

			GTSL::StaticString<64> executionString;

			shaderJson[u8"execution"](executionString);

			if (GTSL::StringView(shaderGroupDomain) == GTSL::StringView(u8"Screen")) {
				executionString = GTSL::ShortString<64>(u8"windowExtent");
			}

			GPipeline::ElementHandle shaderScope = pipeline.DeclareShader(shaderGroupScope, shaderJson[u8"name"]);
			GPipeline::ElementHandle mainFunctionHandle = pipeline.DeclareFunction(shaderScope, u8"void", u8"main");

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

			for(; i < parent->JSON[u8"options"]; ++i) {
				if(parent->JSON[u8"options"][i][u8"domain"] == GTSL::StringView(shaderGroupDomain)) {
					for(; j < parent->JSON[u8"options"][i][u8"subsets"]; ++j) {
						auto shaderClass = GTSL::StaticString<64>(shaderJson[u8"class"]);

						for(uint32 k = 0; k < parent->JSON[u8"options"][i][u8"subsets"][j][u8"sourceClasses"]; ++k) {
							auto s = GTSL::StaticString<64>(parent->JSON[u8"options"][i][u8"subsets"][j][u8"sourceClasses"][k]);
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

			if(!found or !(i < parent->a) or !(j < parent->a[i])) {
				BE_LOG_WARNING(u8"Could not generate shader: ", GTSL::StringView(shaderJson[u8"name"]), u8", for shader group: ", GTSL::StringView(json[u8"name"]), u8", because a shader template for permutation: ", parent->InstanceName, (i < parent->a) ? GTSL::StaticString<128>(u8", option: ") + GTSL::StringView(parent->JSON[u8"options"][i][u8"name"]) : GTSL::StaticString<128>(), (i < parent->a && j < parent->a[i]) ? GTSL::StaticString<128>(u8", subset: ") + GTSL::StringView(parent->JSON[u8"options"][i][u8"subsets"][j][u8"name"]) : GTSL::StaticString<128>(), u8", was not available.");
				return;
			}

			for (auto p : json[u8"parameters"]) {
				GTSL::StaticString<256> code;

				code += u8"const";
				code &= GTSL::StringView(p[u8"type"]);
				code &= GTSL::StringView(p[u8"name"]);
				code &= u8"=";
				code &= u8"pushConstantBlock.shaderParameters[pushConstantBlock.instances[_instanceIndex].shaderGroupIndex]."; code += GTSL::StringView(p[u8"name"]); code += u8";";

				pipeline.AddCodeToFunction(mainFunctionHandle, code);
			}

			pipeline.AddCodeToFunction(mainFunctionHandle, PermutationManager::MakeShaderString(GTSL::StringView(parent->a[i][j]), shaderEntry.rest.element));

			for(auto t : shaderJson[u8"tags"]) {
				tags.EmplaceBack(GTSL::StringView(t[u8"name"]), GTSL::StringView(t[u8"value"]));
			}

			{
				scopes.EmplaceBack(shaderGroupScope);

				// FIX
				{
					auto commonPermutationScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"CommonPermutation");

					Class shaderClass = ShaderClassFromString(shaderJson[u8"class"]);

					GPipeline::ElementHandle subScopeHandle;

					switch (shaderClass) {
						case Class::VERTEX: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"VertexShader"); break;
						case Class::SURFACE: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"FragmentShader"); break;
						case Class::COMPUTE: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"ComputeShader");

						scopes.EmplaceBack(pipeline.GetElementHandle(commonPermutationScopeHandle, u8"ComputeRenderPass"));
						break;
						case Class::RENDER_PASS: break;
						case Class::CLOSEST_HIT: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"ClosestHitShader"); break;
						case Class::RAY_GEN: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"RayGenShader"); break;
						case Class::MISS: subScopeHandle = pipeline.GetElementHandle(GPipeline::GLOBAL_SCOPE, u8"MissShader"); break;
					}

					scopes.EmplaceBack(subScopeHandle);
				}
				// FIX

				scopes.EmplaceBack(shaderScope);
				scopes.EmplaceBack(mainFunctionHandle);

				GTSL::StaticString<512> shaderName;
				GTSL::StaticVector<GTSL::StringView, 16> scopesStrings;

				//make shader name by appending all the names of the scopes that comprise, which allows to easily identify the permutation
				for (auto& e : scopes) {
					auto& n = pipeline.GetElement(e).Name;

					if (n) {
						scopesStrings.EmplaceBack(n);
					}
				}

				shaderName += GTSL::Join{ scopesStrings, u8"." };

				auto targetSemantics = ShaderTypeFromString(parent->JSON[u8"options"][i][u8"subsets"][j][u8"targetSemantics"]);

				auto shaderResult = GenerateShader(pipeline, scopes, targetSemantics, GetTransientAllocator());
				if (!shaderResult) { BE_LOG_WARNING(shaderResult.Get().Second); }
				auto shaderHash = quickhash64(GTSL::Range(shaderResult.Get().First.GetBytes(), reinterpret_cast<const byte*>(shaderResult.Get().First.c_str())));

				if (!loadedShaders.Find(shaderHash)) {
					loadedShaders.Emplace(shaderHash);

					auto [compRes, resultString, shaderBuffer] = compiler_.Compile(shaderResult.Get().First, shaderName, targetSemantics, GAL::ShaderLanguage::GLSL, true,		GetTransientAllocator());

					if (!compRes) { BE_LOG_ERROR(shaderResult.Get().First); BE_LOG_ERROR(resultString); }

					shaderInfoTableFile << shaderHash << shaderInfosFile.GetSize(); //shader info table
					shadersTableFile << shaderHash << shaderPackageFile.GetSize(); //shader table

					shaderInfosFile << GTSL::ShortString<64>(shaderJson[u8"name"]) << static_cast<uint32>(shaderBuffer.GetLength()) << shaderHash;
					shaderInfosFile << 0u; //0 params
					shaderInfosFile << targetSemantics;
					shaderInfosFile << tags.GetLength();
					for (auto& t : tags) {
						shaderInfosFile << t.First << t.Second;
					}

					GTSL::String string(GetTransientAllocator());

					pipeline.MakeJSON(string, scopes); //MAKE JSON

					shaderInfosFile << string; //Debug data

					shaderPackageFile.Write(shaderBuffer);
				}

				shader_group_data_serialize->Shaders.EmplaceBack(shaderGroupUsedShaders.GetLength());
				shaderGroupUsedShaders.EmplaceBack(shaderHash);
			}
		}
	};

	evaluateForPermutation(root_permutation, {GPipeline::GLOBAL_SCOPE}, evaluateForPermutation);

	for(auto& e :shader_group_data_serialize->Instances) {
		shaderGroupsTableFile << GTSL::ShortString<32>(e.Name) << shaderGroupInfosFile.GetSize(); // write offset to shader group for each instance name
	}

	{
		shaderGroupInfosFile << shader_group_data_serialize->Name;

		shaderGroupInfosFile << shaderGroupUsedShaders.GetLength();
		for (auto& e : shaderGroupUsedShaders) { shaderGroupInfosFile << e; }

		shaderGroupInfosFile << shader_group_data_serialize->Parameters.GetLength();
		for (auto& p : shader_group_data_serialize->Parameters) {
			shaderGroupInfosFile << p.Type << p.Name << p.Value;
		}

		shaderGroupInfosFile << shader_group_data_serialize->Tags.GetLength();
		for (auto& p : shader_group_data_serialize->Tags) {
			shaderGroupInfosFile << p.First << p.Second;
		}

		shaderGroupInfosFile << shader_group_data_serialize->Instances.GetLength();
		for (auto& i : shader_group_data_serialize->Instances) {

			shaderGroupInfosFile << i.Name;

			shaderGroupInfosFile << i.Parameters.GetLength();
			for (auto& p : i.Parameters) {
				shaderGroupInfosFile << p.First << p.Second;
			}
		}


		shaderGroupInfosFile << shader_group_data_serialize->VertexElements.GetLength();
		for (auto& e : shader_group_data_serialize->VertexElements) {
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