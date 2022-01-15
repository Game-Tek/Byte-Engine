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

#include <GAL/Serialize.hpp>

#include "ResourceManager.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Render/ShaderGenerator.h"
#include "GTSL/Filesystem.h"

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

	struct TaskShader {

	};

	struct MeshShader {

	};

	struct ComputeShader {

	};

	struct RayGenShader {
		uint8 Recursion = 1;
	};

	struct ClosestHitShader {

	};

	struct MissShader {

	};

	struct AnyHitShader {

	};

	struct IntersectionShader {

	};

	struct CallableShader {

	};

	struct ShaderInfo {
		GTSL::ShortString<32> Name;
		GAL::ShaderType Type; uint64 Hash = 0;
		GTSL::StaticVector<Parameter, 8> Parameters;
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

		ShaderInfo(const ShaderInfo& shader_info) : Name(shader_info.Name), Type(shader_info.Type), Hash(shader_info.Hash), Parameters(shader_info.Parameters), Size(shader_info.Size) {
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
		GTSL::ShortString<32> RenderPassName;

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
		GTSL::ShortString<32> RenderPassName;

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

	template<typename... ARGS>
	void loadShaderGroup(TaskInfo taskInfo, Id shaderGroupName, DynamicTaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS... args) { //TODO: check why can't use ARGS&&
		shaderGroupInfosFile.SetPointer(shaderGroupInfoOffsets[shaderGroupName]);

		ShaderGroupInfo shaderGroupInfo(GetPersistentAllocator());

		shaderGroupInfosFile >> shaderGroupInfo.Name;
		shaderGroupInfosFile >> shaderGroupInfo.RenderPassName;

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

	static GPipeline makeDefaultPipeline() {
		GPipeline pipeline;
		auto descriptorSetBlockHandle = pipeline.Add(GPipeline::ElementHandle(), u8"descriptorSetBlock", GPipeline::LanguageElement::ElementType::SCOPE);
		auto firstDescriptorSetBlockHandle = pipeline.Add(descriptorSetBlockHandle, u8"descriptorSet", GPipeline::LanguageElement::ElementType::SCOPE);
		pipeline.DeclareVariable(firstDescriptorSetBlockHandle, { u8"texture2D[]", u8"textures" });
		pipeline.DeclareVariable(firstDescriptorSetBlockHandle, { u8"image2D[]", u8"images" });
		pipeline.DeclareVariable(firstDescriptorSetBlockHandle, { u8"sampler", u8"s" });

		pipeline.DeclareStruct({}, u8"TextureReference", { { u8"uint32", u8"Instance" } });
		pipeline.DeclareStruct({}, u8"ImageReference", { { u8"uint32", u8"Instance" } });

		pipeline.DeclareRawFunction({}, u8"vec3f", u8"Barycenter", { { u8"vec2f", u8"coords" } }, u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);");
		pipeline.DeclareRawFunction({}, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" } }, u8"return texture(sampler2D(textures[nonuniformEXT(tex.Instance)], s), texCoord);");
		pipeline.DeclareRawFunction({}, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"uvec2", u8"pos" } }, u8"return texelFetch(sampler2D(textures[nonuniformEXT(tex.Instance)], s), ivec2(pos), 0);");
		pipeline.DeclareRawFunction({}, u8"vec4f", u8"Sample", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" } }, u8"return imageLoad(images[nonuniformEXT(img.Instance)], ivec2(pos));");
		pipeline.DeclareRawFunction({}, u8"void", u8"Write", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" }, { u8"vec4f", u8"value" } }, u8"imageStore(images[nonuniformEXT(img.Instance)], ivec2(pos), value);");
		pipeline.DeclareRawFunction({}, u8"float32", u8"X", { { u8"vec4f", u8"vec" } }, u8"return vec.x;");
		pipeline.DeclareRawFunction({}, u8"float32", u8"Y", { { u8"vec4f", u8"vec" } }, u8"return vec.y;");
		pipeline.DeclareRawFunction({}, u8"float32", u8"Z", { { u8"vec4f", u8"vec" } }, u8"return vec.z;");
		pipeline.DeclareRawFunction({}, u8"vec3f", u8"FresnelSchlick", { { u8"float32", u8"cosTheta" }, { u8"vec3f", u8"F0" } }, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
		pipeline.DeclareRawFunction({}, u8"vec3f", u8"Normalize", { { u8"vec3f", u8"a" } }, u8"return normalize(a);");
		pipeline.DeclareRawFunction({}, u8"float32", u8"Sigmoid", { { u8"float32", u8"x" } }, u8"return 1.0 / (1.0 + pow(x / (1.0 - x), -3.0));");
		pipeline.DeclareRawFunction({}, u8"vec3f", u8"WorldPositionFromDepth", { { u8"vec2f", u8"texture_coordinate" }, { u8"float32", u8"depth_from_depth_buffer" }, { u8"mat4f", u8"inverse_projection_matrix" } }, u8"vec4 p = inverse_projection_matrix * vec4(vec3(texture_coordinate * 2.0 - vec2(1.0), depth_from_depth_buffer), 1.0); return p.xyz / p.w;\n");
		pipeline.DeclareRawFunction({}, u8"float32", u8"PI", { }, u8"return 3.14159265359f;");
		pipeline.DeclareRawFunction({}, u8"vec2f", u8"SphericalCoordinates", { { u8"vec3f", u8"v" } }, u8"vec2f uv = vec2(atan(v.z, v.x), asin(v.y)); uv *= vec2(0.1591, 0.3183); uv += 0.5; return uv; ");
		pipeline.DeclareRawFunction({}, u8"float32", u8"DistributionGGX", { { u8"vec3f", u8"N"}, { u8"vec3f", u8"H"}, { u8"float32", u8"roughness"}}, u8"float32 a = roughness * roughness; float32 a2 = a * a; float32 NdotH = max(dot(N, H), 0.0); float32 NdotH2 = NdotH * NdotH; float32 num = a2; float32 denom = (NdotH2 * (a2 - 1.0) + 1.0); denom = PI() * denom * denom; return num / denom;");
		pipeline.DeclareRawFunction({}, u8"float32", u8"GeometrySchlickGGX", { { u8"float32", u8"NdotV"}, { u8"float32", u8"roughness"}}, u8"float32 r = (roughness + 1.0); float32 k = (r * r) / 8.0; float32 num = NdotV; float32 denom = NdotV * (1.0 - k) + k; return num / denom;");
		pipeline.DeclareRawFunction({}, u8"float32", u8"GeometrySmith", { { u8"vec3f", u8"N"}, { u8"vec3f", u8"V"}, { u8"vec3f", u8"L"}, { u8"float32", u8"roughness" } }, u8"float32 NdotV = max(dot(N, V), 0.0); float32 NdotL = max(dot(N, L), 0.0); float32 ggx2 = GeometrySchlickGGX(NdotV, roughness); float32 ggx1 = GeometrySchlickGGX(NdotL, roughness); return ggx1 * ggx2;");

		return pipeline;
	}

	//if (auto fs = shaderJson[u8"functions"]) {
//	for (auto f : fs) {
//		auto& fd = shader.Functions.EmplaceBack();
//
//		fd.Return = f[u8"return"];
//		fd.Name = f[u8"name"];
//
//		pipeline.Add(GPipeline::ElementHandle(), fd.Name, GPipeline::LanguageElement::ElementType::FUNCTION);
//
//		for (auto p : f[u8"params"]) { fd.Parameters.EmplaceBack(p[u8"type"], p[u8"name"]); }
//
//		tokenizeCode(f[u8"code"].GetStringView(), fd.Statements);
//	}
//}
};

struct PermutationManager : Object {
	struct ShaderGenerationData {
		GTSL::StaticVector<GPipeline::ElementHandle, 16> Scopes;
		GTSL::StaticVector<PermutationManager*, 16> Hierarchy;
	};

	PermutationManager(const GTSL::StringView instance_name, const GTSL::StringView class_name) : InstanceName(instance_name), ClassName(class_name) {
		
	}

	void Process(GPipeline* pipeline){
		ShaderGenerationData shader_generation_data;

		Process2(pipeline, shader_generation_data);
	}

	virtual void Process2(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) = 0;

	struct Result {
		GAL::ShaderType TargetSemantics;
		GTSL::StaticVector<GPipeline::ElementHandle, 16> Scopes;
	};
	virtual GTSL::StaticVector<Result, 8> ProcessShader2(GPipeline* pipeline, GTSL::JSONMember shaderGroupJson, GTSL::JSONMember shaderJson, GTSL::StaticVector<PermutationManager*, 16> hierarchy) = 0;

	template<class A>
	PermutationManager* CreateChild(const GTSL::StringView name) {
		return Children.EmplaceBack(GTSL::SmartPointer<A, BE::TAR>(GetTransientAllocator(), name));
	}

	GTSL::StaticVector<Result, 8> ProcessShader(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json) {
		return ProcessShader2(pipeline, shader_group_json, shader_json, {});
	}

	GTSL::StaticVector<GTSL::SmartPointer<PermutationManager, BE::TAR>, 8> Children;
	GTSL::StaticString<64> InstanceName;
	const GTSL::StaticString<64> ClassName;

	template<typename T>
	static T* Find(const GTSL::StringView class_name, const GTSL::Range<PermutationManager**> hierarchy) {
		for (auto& e : hierarchy) {
			if (e->ClassName == class_name) { //pseudo dynamic cast
				return static_cast<T*>(e);
			}
		}

		return nullptr;
	}
};

struct CommonPermutation : PermutationManager {
	CommonPermutation(const GTSL::StringView name) : PermutationManager(name, u8"CommonPermutation") {}

	void Process2(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		vertexShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"VertexShader");
		fragmentShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"FragmentShader");
		computeShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"ComputeShader");
		rayGenShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"RayGenShader");
		closestHitShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"ClosestHitShader");
		anyHitShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"AnyHitShader");
		missShaderScope = pipeline->DeclareScope(GPipeline::ElementHandle(), u8"MissShader");

		pipeline->DeclareRawFunction(fragmentShaderScope, u8"vec2f", u8"GetFragmentPosition", {}, u8"return gl_FragCoord.xy;");
		pipeline->DeclareRawFunction(fragmentShaderScope, u8"float32", u8"GetFragmentDepth", {}, u8"return gl_FragCoord.z;");

		pipeline->DeclareVariable(closestHitShaderScope, { u8"vec2f", u8"hitBarycenter" });
		pipeline->DeclareFunction(closestHitShaderScope, u8"vec3f", u8"GetVertexBarycenter", {}, u8"return Barycenter(hitBarycenter);");

		commonScope = pipeline->DeclareScope({}, u8"Common");
		shader_generation_data.Scopes.EmplaceBack(commonScope);

		pipeline->DeclareStruct(commonScope, u8"globalData", { { u8"uint32", u8"frameIndex" }, {u8"float32", u8"time"} });
		pipeline->DeclareStruct(commonScope, u8"cameraData", { { u8"mat4f", u8"view" }, {u8"mat4f", u8"proj"}, {u8"mat4f", u8"viewInverse"}, {u8"mat4f", u8"projInverse"}, {u8"mat4f", u8"vp"}, {u8"vec4f", u8"worldPosition"} });
		pipeline->DeclareStruct(commonScope, u8"renderPassData", { { u8"ImageReference", u8"Color" }, {u8"ImageReference", u8"Normal" }, { u8"ImageReference", u8"Depth"} });

		auto instanceDataStruct = pipeline->Add(commonScope, u8"instanceData", GPipeline::LanguageElement::ElementType::STRUCT);
		pipeline->DeclareVariable(instanceDataStruct, { u8"mat4x3f", u8"ModelMatrix" });
		auto instanceStructVertexBuffer = pipeline->DeclareVariable(instanceDataStruct, { u8"vertex*", u8"VertexBuffer" });
		auto instanceStructIndexBuffer = pipeline->DeclareVariable(instanceDataStruct, { u8"index*", u8"IndexBuffer" });

		pipeline->DeclareVariable(fragmentShaderScope, { u8"vec4f", u8"Color" });
		pipeline->DeclareVariable(fragmentShaderScope, { u8"vec4f", u8"Normal" });

		auto glPositionHandle = pipeline->DeclareVariable(vertexShaderScope, { u8"vec4f", u8"gl_Position" });
		pipeline->AddMemberDeductionGuide(vertexShaderScope, u8"vertexPosition", { glPositionHandle });

		pipeline->DeclareStruct(rayGenShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });
		pipeline->DeclareStruct(missShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });
		pipeline->DeclareStruct(closestHitShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });

		pipeline->DeclareStruct(rayGenShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"uint64", u8"instances" } });
		pipeline->DeclareStruct(missShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"uint64", u8"instances" } });
		pipeline->DeclareStruct(closestHitShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"instanceData*", u8"instances" } });

		pipeline->DeclareRawFunction(fragmentShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"return vertexIn.vertexTextureCoordinates;");
		pipeline->DeclareRawFunction(fragmentShaderScope, u8"mat4f", u8"GetInverseProjectionMatrix", {}, u8"return pushConstantBlock.camera.projInverse;");
		pipeline->DeclareRawFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceWorldSpacePosition", {}, u8"return vertexIn.worldSpacePosition;");
		pipeline->DeclareRawFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceWorldSpaceNormal", {}, u8"return vertexIn.worldSpaceNormal;");
		pipeline->DeclareRawFunction(fragmentShaderScope, u8"vec3f", u8"GetSurfaceViewSpacePosition", {}, u8"return vertexIn.viewSpacePosition;");
		pipeline->DeclareRawFunction(fragmentShaderScope, u8"vec4f", u8"GetSurfaceViewSpaceNormal", {}, u8"return vec4(vertexIn.viewSpaceNormal, 0);");
		auto fragmentOutputBlockHandle = pipeline->Add(fragmentShaderScope, u8"fragmentOutputBlock", GPipeline::LanguageElement::ElementType::MEMBER);
		auto outColorHandle = pipeline->DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Color" });
		auto outNormalHandle = pipeline->DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Normal" });
		pipeline->AddMemberDeductionGuide(fragmentShaderScope, u8"surfaceColor", { outColorHandle });
		pipeline->AddMemberDeductionGuide(fragmentShaderScope, u8"surfaceNormal", { outNormalHandle });

		pipeline->DeclareRawFunction(vertexShaderScope, u8"vec4f", u8"GetVertexPosition", {}, u8"return vec4(POSITION, 1);");
		pipeline->DeclareRawFunction(vertexShaderScope, u8"vec4f", u8"GetVertexNormal", {}, u8"return vec4(NORMAL, 0);");
		pipeline->DeclareRawFunction(vertexShaderScope, u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"return TEXTURE_COORDINATES;");
		pipeline->DeclareRawFunction(vertexShaderScope, u8"mat4f", u8"GetCameraViewMatrix", {}, u8"return pushConstantBlock.camera.view;");
		pipeline->DeclareRawFunction(vertexShaderScope, u8"mat4f", u8"GetCameraProjectionMatrix", {}, u8"return pushConstantBlock.camera.proj;");

		pipeline->DeclareRawFunction(computeShaderScope, u8"uvec2", u8"GetScreenPosition", {}, u8"return gl_WorkGroupID.xy;");

		pipeline->DeclareRawFunction(rayGenShaderScope, u8"mat4f", u8"GetInverseViewMatrix", {}, u8"return pushConstantBlock.camera.viewInverse;");
		pipeline->DeclareRawFunction(rayGenShaderScope, u8"mat4f", u8"GetInverseProjectionMatrix", {}, u8"return pushConstantBlock.camera.projInverse;");
		pipeline->DeclareRawFunction(rayGenShaderScope, u8"void", u8"TraceRay", { { u8"vec4f", u8"origin" }, { u8"vec4f", u8"direction" } }, u8"traceRayParameterData r = pushConstantBlock.rayTrace.traceRayParameters; traceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, vec3f(origin), r.tMin, vec3f(direction), r.tMax, 0);");
		pipeline->DeclareRawFunction(rayGenShaderScope, u8"vec2u", u8"GetFragmentPosition", {}, u8" return gl_LaunchIDEXT.xy;");
		pipeline->DeclareRawFunction(rayGenShaderScope, u8"vec2f", u8"GetFragmentNormalizedPosition", {}, u8"vec2f pixelCenter = 1vec2f(gl_LaunchIDEXT.xy) + vec2f(0.5f); return pixelCenter / vec2f(gl_LaunchSizeEXT.xy - 1);");

		auto shaderRecordBlockHandle = pipeline->Add(closestHitShaderScope, u8"shaderRecordBlock", GPipeline::LanguageElement::ElementType::MEMBER);
		auto shaderRecordEntry = pipeline->DeclareVariable(shaderRecordBlockHandle, { u8"shaderParametersData*", u8"shaderEntries" });
		pipeline->Add(closestHitShaderScope, u8"surfaceNormal", GPipeline::LanguageElement::ElementType::DISABLED);
		pipeline->DeclareFunction(closestHitShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"instanceData* instance = pushConstantBlock.rayTrace.instances[gl_InstanceCustomIndexEXT]; u16vec3 indices = instance.IndexBuffer[gl_PrimitiveID].indexTri; vec3f barycenter = GetVertexBarycenter(); return instance.VertexBuffer[indices[0]].TEXTURE_COORDINATES * barycenter.x + instance.VertexBuffer[indices[1]].TEXTURE_COORDINATES * barycenter.y + instance.VertexBuffer[indices[2]].TEXTURE_COORDINATES * barycenter.z;");

		shader_generation_data.Hierarchy.EmplaceBack(this);

		for (auto& e : Children) {
			e->Process2(pipeline, shader_generation_data);
		}
	}

	GTSL::StaticVector<Result, 8> ProcessShader2(GPipeline* pipeline, GTSL::JSONMember shaderGroupJson, GTSL::JSONMember shaderJson, GTSL::StaticVector<PermutationManager*, 16> hierarchy) override {
		GTSL::StaticVector<Result, 8> batches;

		hierarchy.EmplaceBack(this);

		for (auto& e : Children) {
			batches.PushBack(e->ProcessShader2(pipeline, shaderGroupJson, shaderJson, hierarchy));
		}

		return batches;
	}

	GPipeline::ElementHandle commonScope;
	GPipeline::ElementHandle vertexShaderScope, fragmentShaderScope, computeShaderScope, rayGenShaderScope, closestHitShaderScope, anyHitShaderScope, missShaderScope;
};

struct VisibilityRenderPassPermutation : PermutationManager {
	VisibilityRenderPassPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"VisibilityRenderPassPermutation") {}

	void Process2(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		visibilityHandle = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"Visibility");

		shader_generation_data.Scopes.EmplaceBack(visibilityHandle);

		pipeline->DeclareStruct(visibilityHandle, u8"PointLightData", { { u8"vec3f", u8"position" }, {u8"float32", u8"radius"} });
		pipeline->DeclareStruct(visibilityHandle, u8"LightingData", { {u8"uint32", u8"pointLightsLength"},  {u8"PointLightData[4]", u8"pointLights"}});

		pushConstantBlockHandle = pipeline->DeclareScope(visibilityHandle, u8"pushConstantBlock");
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"cameraData*", u8"camera" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"LightingData*", u8"lightingData" });
		shaderParametersHandle = pipeline->DeclareVariable(pushConstantBlockHandle, { u8"shaderParametersData*", u8"shaderParameters" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"instanceData*", u8"instance" });

		pipeline->DeclareRawFunction(visibilityHandle, u8"mat4f", u8"GetInstancePosition", {}, u8"return mat4(pushConstantBlock.instance[gl_InstanceIndex].ModelMatrix);");

		pipeline->DeclareFunction(visibilityHandle, u8"vec3f", u8"light", { { u8"vec3f", u8"light_position" }, { u8"vec3f", u8"camera_position" }, { u8"vec3f", u8"surface_world_position" }, { u8"vec3f", u8"surface_normal" }, { u8"vec3f", u8"light_color" }, { u8"vec3f", u8"V" }, { u8"vec3f", u8"color" }, { u8"vec3f", u8"F0" }, { u8"float32", u8"roughness" } }, u8"vec3f L = normalize(light_position - surface_world_position); vec3f H = normalize(V + L); float32 distance = length(light_position - surface_world_position); float32 attenuation = 1.0f / (distance * distance); vec3f radiance = light_color * attenuation; float32 NDF = DistributionGGX(surface_normal, H, roughness); float32 G = GeometrySmith(surface_normal, V, L, roughness); vec3f F = FresnelSchlick(max(dot(H, V), 0.0), F0); vec3f kS = F; vec3f kD = vec3f(1.0) - kS; kD *= 1.0 - 0; vec3f numerator = NDF * G * F; float32 denominator = 4.0f * max(dot(surface_normal, V), 0.0f) * max(dot(surface_normal, L), 0.0f) + 0.0001f; vec3f specular = numerator / denominator; float32 NdotL = max(dot(surface_normal, L), 0.0f); return (kD * color / PI() + specular) * radiance * NdotL;");

		CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

		if (common_permutation) {
			auto vertexSurfaceInterface = pipeline->DeclareScope(visibilityHandle, u8"vertexSurfaceInterface");
			pipeline->DeclareFunction(visibilityHandle, u8"vec3f", u8"GetCameraPosition", {}, u8"return vec3f(pushConstantBlock.camera.worldPosition);");
			auto vertexTextureCoordinatesHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec2f", u8"vertexTextureCoordinates" });
			pipeline->AddMemberDeductionGuide(common_permutation->vertexShaderScope, u8"vertexTextureCoordinates", { { vertexSurfaceInterface }, { vertexTextureCoordinatesHandle } });
			auto vertexViewSpacePositionHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"viewSpacePosition" });
			pipeline->AddMemberDeductionGuide(common_permutation->vertexShaderScope, u8"vertexViewSpacePosition", { { vertexSurfaceInterface }, { vertexViewSpacePositionHandle } });
			auto vertexViewSpaceNormalHandle = pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"viewSpaceNormal" });
			pipeline->AddMemberDeductionGuide(common_permutation->vertexShaderScope, u8"vertexViewSpaceNormal", { { vertexSurfaceInterface }, { vertexViewSpaceNormalHandle } });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"worldSpacePosition" });
			pipeline->DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"worldSpaceNormal" });
		} else {
			BE_LOG_ERROR(u8"Needed CommonPermutation to setup state but not found in hierarchy.")
		}

		shader_generation_data.Hierarchy.EmplaceBack(this);
	}

	GTSL::StaticVector<Result, 8> ProcessShader2(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, GTSL::StaticVector<PermutationManager*, 16> hierarchy) override {
		GTSL::StaticVector<StructElement, 8> shaderParameters;
		GTSL::StaticVector<Result, 8> batches;

		for (auto p : shader_group_json[u8"parameters"]) {
			shaderParameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);
		}

		auto shaderScope = pipeline->DeclareScope(visibilityHandle, shader_json[u8"name"]);
		auto mainFunctionHandle = pipeline->DeclareFunction(shaderScope, u8"void", u8"main");

		{ //add deduction guides for reaching shader parameters
			auto shaderParametersStructHandle = pipeline->DeclareStruct(shaderScope, u8"shaderParametersData", shaderParameters);

			for (auto& e : shaderParameters) {
				pipeline->AddMemberDeductionGuide(shaderScope, shaderParameters.back().Name, { pushConstantBlockHandle, shaderParametersHandle, pipeline->GetElementHandle(shaderParametersStructHandle, e.Name) });
			}			
		}

		if (auto res = shader_json[u8"localSize"]) {
			pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_x", res[0].GetStringView() });
			pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_y", res[1].GetStringView() });
			pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_z", res[2].GetStringView() });
		} else {
			pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_x", u8"1" });
			pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_y", u8"1" });
			pipeline->DeclareVariable(shaderScope, { u8"uint16", u8"group_size_z", u8"1" });
		}

		auto& main = pipeline->GetFunction({ shaderScope }, u8"main");

		//if (auto sv = shader_group_json[u8"shaderVariables"]) {
		//	for (auto e : sv) {
		//		StructElement struct_element(e[u8"type"], e[u8"name"]);
		//
		//		//pipeline.Add(GPipeline::ElementHandle(), struct_element.Name, GPipeline::LanguageElement::ElementType::MEMBER);
		//
		//		if (auto res = e[u8"defaultValue"]) {
		//			struct_element.DefaultValue = res;
		//		}
		//	}
		//}

		switch (Hash(shader_group_json[u8"domain"])) {
		case GTSL::Hash(u8"World"): {
			auto& batch = batches.EmplaceBack();

			batch.Scopes.EmplaceBack(GPipeline::ElementHandle());

			CommonPermutation* common_permutation = Find<CommonPermutation>(u8"CommonPermutation", hierarchy);
			batch.Scopes.EmplaceBack(common_permutation->commonScope);
			batch.Scopes.EmplaceBack(visibilityHandle);

			switch (Hash(shader_json[u8"class"])) {
			case GTSL::Hash(u8"Vertex"): {
				batch.TargetSemantics = GAL::ShaderType::VERTEX;
				batch.Scopes.EmplaceBack(common_permutation->vertexShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);

				tokenizeCode(u8"vertexTextureCoordinates = GetVertexTextureCoordinates(); vertexSurfaceInterface.worldSpacePosition = vec3f(GetInstancePosition() * GetVertexPosition()); vertexSurfaceInterface.worldSpaceNormal = vec3f(GetInstancePosition() * GetVertexNormal());", main.Tokens, GetPersistentAllocator());
				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());

				//todo: analyze if we need to emit compute shader
				break;
			}
			case GTSL::Hash(u8"Surface"): {
				batch.TargetSemantics = GAL::ShaderType::FRAGMENT;
				batch.Scopes.EmplaceBack(common_permutation->fragmentShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);

				tokenizeCode(shader_json[u8"code"], main.Tokens, GetPersistentAllocator());

				tokenizeCode(u8"vec4f BE_COLOR_0 = surfaceColor; surfaceColor = vec4f(0); for(uint32 i = 0; i < pushConstantBlock.lightingData.pointLightsLength; ++i) { PointLightData l = pushConstantBlock.lightingData.pointLights[i]; surfaceColor += vec4f(light(l.position, GetCameraPosition(), GetSurfaceWorldSpacePosition(), GetSurfaceWorldSpaceNormal(), vec3f(1) * l.radius, normalize(GetCameraPosition() - GetSurfaceWorldSpacePosition()), vec3f(BE_COLOR_0), vec3f(0.04f), 0.0f), 0.1); }", main.Tokens, GetPersistentAllocator());

				break;
			}
			case GTSL::Hash(u8"Miss"): {
				batch.TargetSemantics = GAL::ShaderType::COMPUTE;
				batch.Scopes.EmplaceBack(common_permutation->computeShaderScope);
				batch.Scopes.EmplaceBack(shaderScope);
				//todo: emit compute shader for raster
				break;
			}
			default: {
				batches.PopBack(); //remove added batch as no shader was created
				BE_LOG_ERROR(u8"Can't utilize this shader class in this domain.")				
			}
			}
			
			break;
		}
		default: {
			//BE_LOG_ERROR(u8"Unhandled domain.")
				break;
		}
		}

		return batches;
	}

	GPipeline::ElementHandle visibilityHandle, pushConstantBlockHandle, shaderParametersHandle;
};

struct RayTraceRenderPassPermutation : PermutationManager {
	RayTraceRenderPassPermutation(const GTSL::StringView instance_name) : PermutationManager(instance_name, u8"RayTraceRenderPassPermutation") {}

	void Process2(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) override {
		rayTraceHandle = pipeline->DeclareScope(shader_generation_data.Scopes.back(), u8"RayTraceRenderPassPermutation");

		shader_generation_data.Scopes.EmplaceBack(rayTraceHandle);

		const auto pushConstantBlockHandle = pipeline->DeclareScope(rayTraceHandle, u8"pushConstantBlock");
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"globalData*", u8"global" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"cameraData*", u8"camera" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
		pipeline->DeclareVariable(pushConstantBlockHandle, { u8"rayTraceData*", u8"rayTrace" });

		auto* commonPermutation = Find<CommonPermutation>(u8"CommonPermutation", shader_generation_data.Hierarchy);

		auto payloadBlockHandle = pipeline->Add(rayTraceHandle, u8"payloadBlock", GPipeline::LanguageElement::ElementType::SCOPE);
		payloadHandle = pipeline->DeclareVariable(payloadBlockHandle, { u8"vec4f", u8"payload" });
		pipeline->AddMemberDeductionGuide(commonPermutation->closestHitShaderScope, u8"surfaceColor", { payloadHandle });

		shader_generation_data.Hierarchy.EmplaceBack(this);
	}

	GTSL::StaticVector<Result, 8> ProcessShader2(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, GTSL::StaticVector<PermutationManager*, 16> hierarchy) override {
		GTSL::StaticVector<StructElement, 8> shaderParameters;
		GTSL::StaticVector<Result, 8> batches;

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
				commonPermutation->CreateChild<VisibilityRenderPassPermutation>(u8"VisibilityRenderPass");
				//commonPermutation->CreateChild<RayTraceRenderPassPermutation>(u8"RayTraceRenderPass");
			} //todo: parametrize

			GTSL::Buffer deserializer(GetTransientAllocator());
			auto json = Parse(GTSL::StringView(GTSL::Byte(buffer.GetLength()), reinterpret_cast<const utf8*>(buffer.GetData())), deserializer);

			ShaderGroupDataSerialize shaderGroupDataSerialize(GetPersistentAllocator());
			shaderGroupDataSerialize.Name = json[u8"name"];

			auto pipeline = makeDefaultPipeline();

			commonPermutation->Process(&pipeline);

			if (auto jsonVertex = json[u8"vertexElements"]) {
				GTSL::StaticVector<StructElement, 8> vertexElements;

				for (auto a : jsonVertex) {
					auto& t = shaderGroupDataSerialize.VertexElements.EmplaceBack();
					for (auto ve : a) {
						t.EmplaceBack(ve[u8"type"], ve[u8"id"]);
						vertexElements.EmplaceBack(ve[u8"type"], ve[u8"id"]);
					}
				}

				pipeline.DeclareStruct(GPipeline::ElementHandle(), u8"vertex", vertexElements);
				pipeline.DeclareStruct(GPipeline::ElementHandle(), u8"index", { { u8"u16vec3", u8"indexTri" } });
			}

			GTSL::StaticVector<uint64, 16> shaderGroupUsedShaders;

			for (auto s : json[u8"structs"]) {
				GTSL::StaticVector<StructElement, 8> elements;

				for (auto m : s[u8"members"]) {
					elements.EmplaceBack(m[u8"type"], m[u8"name"]);
				}

				pipeline.DeclareStruct(GPipeline::ElementHandle(), s[u8"name"], elements);
			}

			for (auto f : json[u8"functions"]) {
				GTSL::StaticVector<StructElement, 8> elements;
				for (auto p : f[u8"params"]) { elements.EmplaceBack(p[u8"type"], p[u8"name"]); }
				pipeline.DeclareFunction(GPipeline::ElementHandle(), f[u8"type"], f[u8"name"], elements, f[u8"code"]);
			}

			shaderGroupDataSerialize.RenderPassName = json[u8"renderPass"];

			for (auto i : json[u8"instances"]) {
				auto& instance = shaderGroupDataSerialize.Instances.EmplaceBack();
				instance.Name = i[u8"name"];

				for (auto f : i[u8"parameters"]) {
					auto& param = instance.Parameters.EmplaceBack();
					param.First = f[u8"name"];
					param.Second = f[u8"defaultValue"];
				}
			}

			for (auto p : json[u8"parameters"]) {
				shaderGroupDataSerialize.Parameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);
			}

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

				auto shaderBatch = commonPermutation->ProcessShader(&pipeline, json, shaderJson);

				for (auto& sb : shaderBatch) {
					//fillShader(shaderFileBuffer, shader, pipeline, shaderPermutationScopes);
					auto shaderResult = GenerateShader(pipeline, sb.Scopes, sb.TargetSemantics, GetTransientAllocator());
					if (!shaderResult) { BE_LOG_WARNING(shaderResult.Get().Second); }
					auto shaderHash = quickhash64(GTSL::Range(shaderResult.Get().First.GetBytes(), reinterpret_cast<const byte*>(shaderResult.Get().First.c_str())));

					if (loadedShaders.Find(shaderHash)) { continue; }
					loadedShaders.Emplace(shaderHash);

					auto [compRes, resultString, shaderBuffer] = CompileShader(shaderResult.Get().First, s[u8"name"], sb.TargetSemantics, GAL::ShaderLanguage::GLSL, GetTransientAllocator());

					if (!compRes) { BE_LOG_ERROR(shaderResult.Get().First); BE_LOG_ERROR(resultString); }

					shaderInfoTableFile << shaderHash << shaderInfosFile.GetSize(); //shader info table
					shadersTableFile << shaderHash << shaderPackageFile.GetSize(); //shader table

					shaderInfosFile << GTSL::ShortString<32>(s[u8"name"]) << static_cast<uint32>(shaderBuffer.GetLength()) << shaderHash;
					shaderInfosFile << 0; //0 params
					shaderInfosFile << sb.TargetSemantics;

					shaderPackageFile.Write(shaderBuffer);

					shaderGroupDataSerialize.Shaders.EmplaceBack(shaderGroupUsedShaders.GetLength());
					shaderGroupUsedShaders.EmplaceBack(shaderHash);
				}
			}

			shaderGroupsTableFile << shaderGroupDataSerialize.Name << shaderGroupInfosFile.GetSize();

			{
				shaderGroupInfosFile << shaderGroupDataSerialize.Name;
				shaderGroupInfosFile << shaderGroupDataSerialize.RenderPassName;

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