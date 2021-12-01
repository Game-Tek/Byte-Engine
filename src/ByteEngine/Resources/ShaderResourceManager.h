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
	ShaderResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"ShaderResourceManager"), shaderGroupInfoOffsets(8, GetPersistentAllocator()), shaderInfoOffsets(8, GetPersistentAllocator()), shaderOffsets(8, GetPersistentAllocator())
	{
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

		if(!(shaderPackageFile.GetSize() && shaderGroupsTableFile.GetSize() && shaderInfoTableFile.GetSize() && shadersTableFile.GetSize() && shaderInfosFile.GetSize() && shaderGroupInfosFile.GetSize())) {
			shaderPackageFile.Resize(0);
			shaderGroupsTableFile.Resize(0);
			shaderInfoTableFile.Resize(0);
			shadersTableFile.Resize(0);
			shaderInfosFile.Resize(0);
			shaderGroupInfosFile.Resize(0);
			created = true;
		}

		if (created) {
			GTSL::FileQuery shaderGroupFileQuery;

			GTSL::KeyMap<Id, BE::TAR> loadedShaders(64, GetTransientAllocator());

			while (auto fileRef = shaderGroupFileQuery.DoQuery(GetResourcePath(u8"*ShaderGroup.json"))) {
				GTSL::File shaderGroupFile;
				shaderGroupFile.Open(GetResourcePath(fileRef.Get()), GTSL::File::READ, false);

				GTSL::Buffer buffer(shaderGroupFile.GetSize(), 16, GetTransientAllocator());
				shaderGroupFile.Read(buffer);

				GTSL::Buffer<BE::TAR> deserializer(GetTransientAllocator());
				auto json = GTSL::Parse(GTSL::StringView(GTSL::Byte(buffer.GetLength()), reinterpret_cast<const utf8*>(buffer.GetData())), deserializer);

				ShaderGroupDataSerialize shaderGroupDataSerialize(GetPersistentAllocator());
				shaderGroupDataSerialize.Name = json[u8"name"];

				GPipeline pipeline;
				pipeline.descriptors.EmplaceBack().EmplaceBack(u8"uniform texture2D textures[]");
				pipeline.descriptors.back().EmplaceBack(u8"uniform image2D images[]");
				pipeline.descriptors.back().EmplaceBack(u8"uniform sampler s");

				pipeline.Outputs.EmplaceBack(u8"vec4f", u8"Color");
				pipeline.Outputs.EmplaceBack(u8"vec4f", u8"Normal");

				pipeline.ShaderRecord[GAL::HIT_TABLE_INDEX].EmplaceBack(u8"ptr_t*", u8"MaterialData");

				pipeline.TargetSemantics = GTSL::ShortString<32>(u8"raster");
				pipeline.Interface.EmplaceBack(u8"vec2f", u8"textureCoordinates");
				pipeline.Interface.EmplaceBack(u8"vec3f", u8"viewSpacePosition");
				pipeline.Interface.EmplaceBack(u8"vec3f", u8"viewSpaceNormal");

				for (auto s : json[u8"structs"]) {
					auto& st = pipeline.Structs.EmplaceBack();
					st.Name = s[u8"name"];

					for (auto m : s[u8"members"]) {
						st.Members.EmplaceBack(m[u8"type"], m[u8"name"]);
					}
				}

				for (auto l : json[u8"layers"]) {
					pipeline.Layers.EmplaceBack(l[u8"type"], l[u8"name"]);
				}

				for (auto f : json[u8"functions"]) {
					auto& fs = pipeline.Functions.EmplaceBack();
					fs.Name = f[u8"name"];
					fs.Return = f[u8"type"];

					for (auto p : f[u8"params"]) {
						fs.Parameters.EmplaceBack(p[u8"type"], p[u8"name"]);
					}

					for (auto s : f[u8"statements"]) {
						parseStatement(s, fs.Statements.EmplaceBack(GetPersistentAllocator()), 0);
					}
				}

				if (auto vertexElements = json[u8"vertexElements"]) {
					for (auto ve : vertexElements) {
						auto& e = pipeline.VertexElements.EmplaceBack();
						e.Identifier = ve[u8"id"];

						switch (Hash(ve[u8"type"])) {
						case GTSL::Hash(u8"float3"):
							e.Type = GAL::ShaderDataType::FLOAT3;
							break;
						case GTSL::Hash(u8"float2"):
							e.Type = GAL::ShaderDataType::FLOAT2;
							break;
						}
					}
				}

				for (auto g : json[u8"groups"]) {
					auto& group = shaderGroupDataSerialize.Groups.EmplaceBack(GetPersistentAllocator());

					for (auto p : g[u8"parameters"]) {
						group.Parameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);
						pipeline.parameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);
					}

					for (auto i : g[u8"instances"]) {
						auto& instance = group.Instances.EmplaceBack();
						instance.Name = i[u8"name"];

						for (auto f : i[u8"parameters"]) {
							auto& param = instance.Parameters.EmplaceBack();
							param.First = f[u8"name"];
							param.Second = f[u8"defaultValue"];
						}
					}

					for (auto t : g[u8"tags"]) {
						group.Tags.EmplaceBack(t);
					}

					for (auto s : g[u8"shaders"]) {
						GTSL::File shaderFile; shaderFile.Open(GetResourcePath(s[u8"name"], u8"json"));
						GTSL::Buffer shaderFileBuffer(shaderFile.GetSize(), 16, GetTransientAllocator()); shaderFile.Read(shaderFileBuffer);

						if (loadedShaders.Find(Id(s[u8"name"]))) { continue; }

						loadedShaders.Emplace(Id(s[u8"name"]));

						auto [shaderString, genShader] = GenerateShader(GTSL::StringView(GTSL::Byte(shaderFileBuffer.GetLength()), reinterpret_cast<const utf8*>(shaderFileBuffer.GetData())), pipeline);

						auto [compRes, resultString, shaderBuffer] = CompileShader(shaderString, s[u8"name"], genShader.TargetSemantics, GAL::ShaderLanguage::GLSL, GetTransientAllocator());

						if (!compRes) {
							BE_LOG_ERROR(shaderString);
							BE_LOG_ERROR(resultString);
						}

						shaderInfoTableFile << GTSL::ShortString<32>(s[u8"name"]) << shaderInfosFile.GetSize(); //shader info table
						shadersTableFile << GTSL::ShortString<32>(s[u8"name"]) << shaderPackageFile.GetSize(); //shader table

						shaderInfosFile << GTSL::ShortString<32>(s[u8"name"]) << static_cast<uint32>(shaderBuffer.GetLength());
						shaderInfosFile << 0; //0 params
						shaderInfosFile << genShader.TargetSemantics;

						switch (genShader.TargetSemantics) {
						case GAL::ShaderType::VERTEX: {
							shaderInfosFile << pipeline.VertexElements.GetLength();

							for (auto& e : pipeline.VertexElements) {
								shaderInfosFile << e.Type << e.Identifier;
							}

							break;
						}
						case GAL::ShaderType::FRAGMENT: break;
						case GAL::ShaderType::COMPUTE: break;
						case GAL::ShaderType::RAY_GEN: break;
						case GAL::ShaderType::ANY_HIT: break;
						case GAL::ShaderType::CLOSEST_HIT: break;
						case GAL::ShaderType::MISS: break;
						case GAL::ShaderType::INTERSECTION: break;
						case GAL::ShaderType::CALLABLE: break;
						case GAL::ShaderType::TASK: break;
						case GAL::ShaderType::MESH: break;
						}

						shaderPackageFile.Write(shaderBuffer);

						group.Shaders.EmplaceBack(s[u8"name"]);
					}
				}

				shaderGroupsTableFile << shaderGroupDataSerialize.Name << shaderGroupInfosFile.GetSize();
				shaderGroupInfosFile << shaderGroupDataSerialize;
			}
		}

		{
			shaderGroupsTableFile.SetPointer(0);

			uint32 offset = 0;
			while (offset < shaderGroupsTableFile.GetSize()) {
				GTSL::ShortString<32> name; uint64 position;
				shaderGroupsTableFile >> name >> position;
				offset += 32 + 8;
				shaderGroupInfoOffsets.Emplace(Id(name), position);
			}
		}

		{
			shaderInfoTableFile.SetPointer(0);
			uint32 offset = 0;
			while (offset < shaderInfoTableFile.GetSize()) {
				GTSL::ShortString<32> name; uint64 position;
				shaderInfoTableFile >> name >> position;
				offset += 32 + 8;
				shaderInfoOffsets.Emplace(Id(name), position);
			}
		}

		{
			shadersTableFile.SetPointer(0);
			uint32 offset = 0;
			while (offset < shadersTableFile.GetSize()) {
				GTSL::ShortString<32> name; uint64 position;
				shadersTableFile >> name >> position;
				offset += 32 + 8;
				shaderOffsets.Emplace(Id(name), position);
			}
		}
	}

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

	struct VertexShader {
		GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> VertexElements;

		friend void Insert(const VertexShader& vertex_shader, auto& buffer) {
			Insert(vertex_shader.VertexElements, buffer);
		}

		friend void Extract(VertexShader& vertex_shader, auto& buffer) {
			Extract(vertex_shader.VertexElements, buffer);
		}
	};

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
		GAL::ShaderType Type;
		GTSL::StaticVector<Parameter, 8> Parameters;

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
			case GAL::ShaderType::RAY_GEN: break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			default: __debugbreak();
			}
		}

		ShaderInfo(const ShaderInfo& shader_info) : Name(shader_info.Name), Type(shader_info.Type), Parameters(shader_info.Parameters) {
			switch (Type) {
			case GAL::ShaderType::VERTEX: ::new(&VertexShader) struct VertexShader(shader_info.VertexShader); break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: ::new(&FragmentShader) struct FragmentShader(shader_info.FragmentShader); break;
			case GAL::ShaderType::COMPUTE: ::new(&ComputeShader) struct ComputeShader(shader_info.ComputeShader); break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: break;
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
	};

	struct Shader : ShaderInfo {
		uint32 Size = 0;

		template<class ALLOC>
		friend void Insert(const Shader& shader, GTSL::Buffer<ALLOC>& buffer) {
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
		friend void Extract(Shader& shader, GTSL::Buffer<ALLOC>& buffer) {
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

		Shader() {}

		Shader(const Shader& shader) : ShaderInfo(shader), Size(shader.Size) {
		}

		Shader& operator=(const Shader& other) {
			Size = other.Size;
			Name = other.Name;
			Type = other.Type;
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
			case GAL::ShaderType::RAY_GEN: break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			}

			return *this;
		}
	};

	struct ShaderGroupData : Data {
		ShaderGroupData(const BE::PAR& allocator) : Groups(allocator) {}

		GTSL::ShortString<32> Name;

		struct Group : Object {
			Group(const BE::PAR& allocator) : Shaders(allocator), Instances(allocator), Parameters(allocator), Tags(allocator) {}

			GTSL::Vector<GTSL::ShortString<32>, BE::PAR> Shaders;
			GTSL::Vector<ShaderGroupInstance, BE::PAR> Instances;
			GTSL::Vector<Parameter, BE::PAR> Parameters;
			GTSL::Vector<GTSL::ShortString<32>, BE::PAR> Tags;

			friend auto operator<<(auto& buffer, const Group& group) -> decltype(buffer)& {
				buffer << group.Parameters.GetLength();
				for (auto& p : group.Parameters) {
					buffer << p.Type << p.Name << p.Value;
				}

				buffer << group.Instances.GetLength();
				for (auto& i : group.Instances) {
					buffer << i.Name;

					buffer << i.Parameters.GetLength();
					for (auto& p : i.Parameters) {
						buffer << p.First << p.Second;
					}
				}

				buffer << group.Shaders.GetLength();
				for (auto& e : group.Shaders) {
					buffer << e;
				}

				buffer << group.Tags.GetLength();
				for (auto& t : group.Tags) {
					buffer << t;
				}

				return buffer;
			}
		};
		GTSL::Vector<Group, BE::PAR> Groups;

		//GTSL::StaticVector<GTSL::ShortString<32>, 16> Shaders;
	};

	struct ShaderGroupDataSerialize : ShaderGroupData, Object {
		ShaderGroupDataSerialize(const BE::PAR& allocator) : ShaderGroupData(allocator) {}

		INSERT_START(ShaderGroupDataSerialize) {
				Insert(insertInfo.Name, buffer);
			buffer << insertInfo.Groups;
		}

		EXTRACT_START(ShaderGroupDataSerialize) {
			Extract(extractInfo.Name, buffer);

			uint32 length;
			buffer >> length;

			for (uint32 i = 0; i < length; ++i) {
				buffer >> extractInfo.Groups[i];
			}
		}

		friend auto operator<<(auto& buffer, const ShaderGroupDataSerialize& i) {
			buffer << i.Name << i.Groups;
		}
	};

	struct ShaderGroupInfo {
		ShaderGroupInfo(const BE::PAR& allocator) : Groups(allocator) {}

		GTSL::ShortString<32> Name;

		struct Group {
			Group(const BE::PAR& allocator) : Shaders(allocator), Instances(allocator), Parameters(allocator), Tags(allocator) {}

			GTSL::Vector<Shader, BE::PAR> Shaders;
			GTSL::Vector<ShaderGroupInstance, BE::PAR> Instances;
			GTSL::Vector<Parameter, BE::PAR> Parameters;
			GTSL::Vector<GTSL::ShortString<32>, BE::PAR> Tags;
		};
		GTSL::Vector<Group, BE::PAR> Groups;
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
	GTSL::HashMap<Id, uint64, BE::PersistentAllocatorReference> shaderGroupInfoOffsets, shaderInfoOffsets, shaderOffsets;
	mutable GTSL::ReadWriteMutex mutex;

	template<typename... ARGS>
	void loadShaderGroup(TaskInfo taskInfo, Id shaderGroupName, DynamicTaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS... args) { //TODO: check why can't use ARGS&&
		shaderGroupInfosFile.SetPointer(shaderGroupInfoOffsets[shaderGroupName]);

		ShaderGroupInfo shaderGroupInfo(GetPersistentAllocator());

		shaderGroupInfosFile >> shaderGroupInfo.Name;

		uint32 groupCount = 0;
		shaderGroupInfosFile >> groupCount;

		for (uint32 g = 0; g < groupCount; ++g) {
			auto& group = shaderGroupInfo.Groups.EmplaceBack(GetPersistentAllocator());

			uint32 parameterCount;
			shaderGroupInfosFile >> parameterCount;

			for (uint32 p = 0; p < parameterCount; ++p) {
				auto& parameter = group.Parameters.EmplaceBack();
				shaderGroupInfosFile >> parameter.Type >> parameter.Name >> parameter.Value;
			}

			uint32 instanceCount;
			shaderGroupInfosFile >> instanceCount;

			for (uint32 i = 0; i < instanceCount; ++i) {
				auto& instance = group.Instances.EmplaceBack();
				shaderGroupInfosFile >> instance.Name;

				uint32 params = 0;
				shaderGroupInfosFile >> params;

				for (uint32 p = 0; p < params; ++p) {
					auto& param = instance.Parameters.EmplaceBack();
					shaderGroupInfosFile >> param.First >> param.Second;
				}
			}

			uint32 shaderCount;
			shaderGroupInfosFile >> shaderCount;

			for (uint32 s = 0; s < shaderCount; ++s) {
				GTSL::ShortString<32> shaderName;
				shaderGroupInfosFile >> shaderName;

				auto& shader = group.Shaders.EmplaceBack();

				{
					shaderInfosFile.SetPointer(shaderInfoOffsets[Id(shaderName)]);
					shaderInfosFile >> shader.Name >> shader.Size;

					uint32 paramCount = 0;
					shaderInfosFile >> paramCount;

					for (uint32 p = 0; p < paramCount; ++p) {
						auto& parameter = shader.Parameters.EmplaceBack();
						shaderInfosFile >> parameter.Name >> parameter.Type >> parameter.Value;
					}

					GAL::ShaderType shaderType;
					shaderInfosFile >> shaderType;

					shader.SetType(shaderType);

					switch (shader.Type) {
					case GAL::ShaderType::VERTEX: {
						uint32 vertexElementsCount;
						shaderInfosFile >> vertexElementsCount;

						for (uint32 ve = 0; ve < vertexElementsCount; ++ve) {
							auto& vertexElement = shader.VertexShader.VertexElements.EmplaceBack();
							shaderInfosFile >> vertexElement.Type >> vertexElement.Identifier;
						}

						break;
					}
					}
				}
			}

			uint32 tagCount;
			shaderGroupInfosFile >> tagCount;

			for (uint32 t = 0; t < tagCount; ++t) {
				auto& tag = group.Tags.EmplaceBack();
				shaderGroupInfosFile >> tag;
			}
		}

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(shaderGroupInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadShaders(TaskInfo taskInfo, ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer, DynamicTaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, ARGS... args) {
		uint32 offset = 0;

		for (const auto& g : shader_group_info.Groups) {
			for (const auto& s : g.Shaders) {
				shaderPackageFile.SetPointer(shaderOffsets[Id(s.Name)]);
				shaderPackageFile.Read(s.Size, offset, buffer);
				offset += s.Size;
			}
		}

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
	};
};
