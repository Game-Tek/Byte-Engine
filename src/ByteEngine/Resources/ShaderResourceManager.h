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
#include "ByteEngine/Render/ShaderGenerator.h"
#include "ByteEngine/Render/ShaderGenerator.h"
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

		GTSL::KeyMap<Id, BE::TAR> loadedShaders(128, GetTransientAllocator());

		if (created) {
			GTSL::FileQuery shaderGroupFileQuery;

			while (auto fileRef = shaderGroupFileQuery.DoQuery(GetResourcePath(u8"*ShaderGroup.json"))) {
				GTSL::File shaderGroupFile;
				shaderGroupFile.Open(GetResourcePath(fileRef.Get()), GTSL::File::READ, false);

				GTSL::Buffer buffer(shaderGroupFile.GetSize(), 16, GetTransientAllocator());
				shaderGroupFile.Read(buffer);

				GTSL::Buffer<BE::TAR> deserializer(GetTransientAllocator());
				auto json = GTSL::Parse(GTSL::StringView(GTSL::Byte(buffer.GetLength()), reinterpret_cast<const utf8*>(buffer.GetData())), deserializer);

				ShaderGroupDataSerialize shaderGroupDataSerialize(GetPersistentAllocator());
				shaderGroupDataSerialize.Name = json[u8"name"];

				GPipeline pipeline = makeDefaultPipeline();

				for (auto s : json[u8"structs"]) {
					auto& st = pipeline.Structs.EmplaceBack();
					st.Name = s[u8"name"];

					pipeline.Add(GPipeline::ElementHandle(), st.Name, GPipeline::LanguageElement::Type::TYPE);

					for (auto m : s[u8"members"]) {
						st.Members.EmplaceBack(m[u8"type"], m[u8"name"]);
					}
				}

				for (auto l : json[u8"layers"]) {
					pipeline.Layers.EmplaceBack(l[u8"type"], l[u8"name"]);
				}

				if (auto vertexElements = json[u8"vertexElements"]) {
					for (auto ve : vertexElements) {
						GAL::Pipeline::VertexElement e;
						e.Identifier = ve[u8"id"];

						switch (Hash(ve[u8"type"])) {
						case GTSL::Hash(u8"float3"):
							e.Type = GAL::ShaderDataType::FLOAT3;
							break;
						case GTSL::Hash(u8"float2"):
							e.Type = GAL::ShaderDataType::FLOAT2;
							break;
						}

						pipeline.DeclareVertexElement(e);
					}
				}

				for (auto f : json[u8"functions"]) {
					auto& fs = pipeline.Functions.EmplaceBack();
					fs.Name = f[u8"name"];
					fs.Return = f[u8"type"];

					pipeline.Add(GPipeline::ElementHandle(), fs.Name, GPipeline::LanguageElement::Type::FUNCTION);

					for (auto p : f[u8"params"]) {
						fs.Parameters.EmplaceBack(p[u8"type"], p[u8"name"]);
					}

					pipeline.DeclareFunction({}, fs.Return, fs.Name, fs.Parameters, f[u8"code"]);
				}

				for (auto g : json[u8"groups"]) {
					auto& group = shaderGroupDataSerialize.Groups.EmplaceBack(GetPersistentAllocator());

					for (auto p : g[u8"parameters"]) {
						group.Parameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);
						pipeline.parameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);

						pipeline.Add(GPipeline::ElementHandle(), p[u8"name"], GPipeline::LanguageElement::Type::MEMBER);
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

						auto shader = makeShader(shaderFileBuffer, pipeline);

						GAL::ShaderType targetSemantics; GPipeline::ElementHandle scope;

						switch (shader.Type) {
						case ::Shader::Class::VERTEX: targetSemantics = GAL::ShaderType::VERTEX; scope = pipeline.VertexShaderScope; break;
						case ::Shader::Class::SURFACE: targetSemantics = GAL::ShaderType::FRAGMENT; scope = pipeline.FragmentShaderScope; break;
						case ::Shader::Class::COMPUTE: break;
						case ::Shader::Class::RENDER_PASS: break;
						case ::Shader::Class::RAY_GEN: break;
						case ::Shader::Class::MISS: break;
						}

						auto shaderString = GenerateShader(shader, pipeline, scope, targetSemantics);

						auto [compRes, resultString, shaderBuffer] = CompileShader(shaderString, s[u8"name"], targetSemantics, GAL::ShaderLanguage::GLSL, GetTransientAllocator());

						if (!compRes) {
							BE_LOG_ERROR(shaderString);
							BE_LOG_ERROR(resultString);
						}

						shaderInfoTableFile << GTSL::ShortString<32>(s[u8"name"]) << shaderInfosFile.GetSize(); //shader info table
						shadersTableFile << GTSL::ShortString<32>(s[u8"name"]) << shaderPackageFile.GetSize(); //shader table

						shaderInfosFile << GTSL::ShortString<32>(s[u8"name"]) << static_cast<uint32>(shaderBuffer.GetLength());
						shaderInfosFile << 0; //0 params
						shaderInfosFile << targetSemantics;

						switch (targetSemantics) {
						case GAL::ShaderType::VERTEX: {
							shaderInfosFile << static_cast<uint32>(pipeline.GetVertexElements().ElementCount());

							for (auto& e : pipeline.GetVertexElements()) {
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

	void GetRayTracePipeline() {
		GTSL::FileQuery rtPipelineFileQuery;

		struct RayTracePipeline {
			GTSL::StaticString<64> Name;
			StructElement Payload;
		};

		while (auto fileRef = rtPipelineFileQuery.DoQuery(GetResourcePath(u8"*Pipeline.json"))) {
			GTSL::File shaderGroupFile;
			shaderGroupFile.Open(GetResourcePath(fileRef.Get()), GTSL::File::READ, false);

			GTSL::Buffer buffer(shaderGroupFile.GetSize(), 16, GetTransientAllocator());
			shaderGroupFile.Read(buffer);

			GTSL::Buffer<BE::TAR> deserializer(GetTransientAllocator());
			auto json = GTSL::Parse(GTSL::StringView(GTSL::Byte(buffer.GetLength()), reinterpret_cast<const utf8*>(buffer.GetData())), deserializer);

			json[u8"name"];
			json[u8"payload"];

			for (auto sg : json[u8"shaderGroups"]) {
				sg[u8"name"];
			}
		}
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

	GPipeline makeDefaultPipeline() {
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

		pipeline.VertexShaderScope = pipeline.Add({}, u8"VertexShader", GPipeline::LanguageElement::Type::SCOPE);
		pipeline.FragmentShaderScope = pipeline.Add({}, u8"FragmentShader", GPipeline::LanguageElement::Type::SCOPE);
		pipeline.ComputeShaderScope = pipeline.Add({}, u8"ComputeShader", GPipeline::LanguageElement::Type::SCOPE);
		pipeline.RayGenShaderScope = pipeline.Add({}, u8"RayGenShader", GPipeline::LanguageElement::Type::SCOPE);
		pipeline.ClosestHitShaderScope = pipeline.Add({}, u8"ClosestHitShader", GPipeline::LanguageElement::Type::SCOPE);

		pipeline.DeclareFunction(pipeline.VertexShaderScope, u8"void", u8"main"); //main
		pipeline.DeclareFunction(pipeline.FragmentShaderScope, u8"void", u8"main"); //main

		pipeline.DeclareRawFunction(pipeline.FragmentShaderScope, u8"vec2f", u8"GetFragmentPosition", {}, u8"return gl_FragCoord.xy;");
		pipeline.DeclareRawFunction(pipeline.FragmentShaderScope, u8"float32", u8"GetFragmentDepth", {}, u8"return gl_FragCoord.z;");
		pipeline.DeclareRawFunction(pipeline.FragmentShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"return vertexIn.textureCoordinates;");
		pipeline.DeclareRawFunction(pipeline.FragmentShaderScope, u8"mat4f", u8"GetInverseProjectionMatrix", {}, u8"return invocationInfo.camera.projInverse;");
		pipeline.DeclareRawFunction(pipeline.FragmentShaderScope, u8"vec3f", u8"GetVertexViewSpacePosition", {}, u8"return vertexIn.viewSpacePosition;");
		pipeline.DeclareRawFunction(pipeline.FragmentShaderScope, u8"vec4f", u8"GetSurfaceViewSpaceNormal", {}, u8"return vec4(vertexIn.viewSpaceNormal, 0);");

		pipeline.DeclareVariable(pipeline.FragmentShaderScope, u8"surfaceColor", u8"out_Color");
		pipeline.DeclareVariable(pipeline.FragmentShaderScope, u8"surfaceNormal", u8"out_Normal");
		pipeline.DeclareVariable(pipeline.FragmentShaderScope, u8"surfacePosition", u8"out_Position");
		pipeline.DeclareVariable(pipeline.FragmentShaderScope, u8"albedo", u8"invocationInfo.shaderParameters.albedo");
		
		pipeline.DeclareRawFunction(pipeline.VertexShaderScope, u8"vec4f", u8"GetVertexPosition", {}, u8"return vec4(in_POSITION, 1);");
		pipeline.DeclareRawFunction(pipeline.VertexShaderScope, u8"vec4f", u8"GetVertexNormal", {}, u8"return vec4(in_NORMAL, 0);");

		pipeline.DeclareRawFunction(pipeline.VertexShaderScope, u8"mat4f", u8"GetInstancePosition", {}, u8"return invocationInfo.instance.ModelMatrix;");
		pipeline.DeclareRawFunction(pipeline.VertexShaderScope, u8"mat4f", u8"GetCameraViewMatrix", {}, u8"return invocationInfo.camera.view;");
		pipeline.DeclareRawFunction(pipeline.VertexShaderScope, u8"mat4f", u8"GetCameraProjectionMatrix", {}, u8"return invocationInfo.camera.proj;");

		pipeline.DeclareVariable(pipeline.VertexShaderScope, u8"vertexTextureCoordinates", u8"vertexOut.textureCoordinates");
		pipeline.DeclareVariable(pipeline.VertexShaderScope, u8"vertexViewSpacePosition", u8"vertexOut.viewSpacePosition");
		pipeline.DeclareVariable(pipeline.VertexShaderScope, u8"vertexViewSpaceNormal", u8"vertexOut.viewSpaceNormal");
		pipeline.DeclareVariable(pipeline.VertexShaderScope, u8"vertexPosition", u8"gl_Position");

		pipeline.DeclareRawFunction(pipeline.ComputeShaderScope, u8"uvec2", u8"GetScreenPosition", {}, u8"return gl_WorkGroupID.xy;");

		pipeline.DeclareRawFunction(pipeline.VertexShaderScope, u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"return in_TEXTURE_COORDINATES;");

		pipeline.DeclareRawFunction(pipeline.RayGenShaderScope, u8"void", u8"TraceRay", { { u8"vec3", u8"origin" }, { u8"vec3", u8"direction" } }, u8"rayTraceDataPointer r = invocationInfo.RayDispatchData;\ntraceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, origin, r.tMin, direction, r.tMax, 0);");
		pipeline.DeclareRawFunction(pipeline.RayGenShaderScope, u8"vec2f", u8"GetFragmentPosition", {}, u8"const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5f);\nreturn pixelCenter / vec2(gl_LaunchSizeEXT.xy);");

		pipeline.DeclareVariable(pipeline.ClosestHitShaderScope, u8"hitBarycenter", u8"hitBarycenter");
		pipeline.DeclareFunction(pipeline.ClosestHitShaderScope, u8"vec3f", u8"GetVertexBarycenter", {}, u8"return Barycenter(hitBarycenter);");
		pipeline.DeclareRawFunction(pipeline.ClosestHitShaderScope, u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"StaticMeshPointer instance = shaderEntries[gl_InstanceCustomIndexEXT]; uint16_t indices[3] = instance.IndexBuffer[3 * gl_PrimitiveID]; vertex vertices[3] = vertex[](instance.VertexBuffer[indeces[0]], instance.VertexBuffer[indeces[1]], instance.VertexBuffer[indeces[2]]); vec2 barycenter = GetVertexBarycenter(); return vertices[0].TexCoords * barycenter.x + vertices[1].TexCoords * barycenter.y + vertices[2].TexCoords * barycenter.z;");

		return pipeline;
	}

	::Shader makeShader(const GTSL::Buffer<BE::TAR>& shaderFileBuffer, GPipeline& pipeline) {
		GTSL::Buffer json_deserializer(BE::TAR(u8"GenerateShader"));
		auto shaderJson = Parse(GTSL::StringView(GTSL::Byte(shaderFileBuffer.GetLength()), reinterpret_cast<const utf8*>(shaderFileBuffer.GetData())), json_deserializer);

		::Shader::Class shaderClass; GPipeline::ElementHandle scope; GAL::ShaderType shaderType; //TODO

		switch (Hash(shaderJson[u8"class"])) {
		case GTSL::Hash(u8"Vertex"): shaderClass = ::Shader::Class::VERTEX; scope = pipeline.VertexShaderScope; break;
		case GTSL::Hash(u8"Surface"): shaderClass = ::Shader::Class::SURFACE; scope = pipeline.FragmentShaderScope; break;
		case GTSL::Hash(u8"Compute"): shaderClass = ::Shader::Class::COMPUTE; scope = pipeline.ComputeShaderScope; break;
		case GTSL::Hash(u8"RayGen"): shaderClass = ::Shader::Class::RAY_GEN; scope = pipeline.RayGenShaderScope; break;
		case GTSL::Hash(u8"Miss"): shaderClass = ::Shader::Class::MISS; scope = pipeline.MissShaderScope; break;
		}

		::Shader shader(shaderJson[u8"name"], shaderClass);

		if (shaderClass == ::Shader::Class::COMPUTE) {
			if (auto res = shaderJson[u8"localSize"]) {
				shader.SetThreadSize({ static_cast<uint16>(res[0].GetUint()), static_cast<uint16>(res[1].GetUint()), static_cast<uint16>(res[2].GetUint()) });
			}
			else {
				shader.SetThreadSize({ 1, 1, 1 });
			}
		}

		if (auto sv = shaderJson[u8"shaderVariables"]) {
			for (auto e : sv) {
				StructElement struct_element(e[u8"type"], e[u8"name"]);

				pipeline.Add(GPipeline::ElementHandle(), struct_element.Name, GPipeline::LanguageElement::Type::MEMBER);

				if (auto res = e[u8"defaultValue"]) {
					struct_element.DefaultValue = res;
				}

				shader.ShaderParameters.EmplaceBack(struct_element);
			}
		}

		if (auto tr = shaderJson[u8"transparency"]) {
			shader.Transparency = tr.GetBool();
		}

		if (auto fs = shaderJson[u8"functions"]) {
			for (auto f : fs) {
				auto& fd = shader.Functions.EmplaceBack();

				fd.Return = f[u8"return"];
				fd.Name = f[u8"name"];

				pipeline.Add(GPipeline::ElementHandle(), fd.Name, GPipeline::LanguageElement::Type::FUNCTION);

				for (auto p : f[u8"params"]) {
					fd.Parameters.EmplaceBack(p[u8"type"], p[u8"name"]);
				}

				parseCode(f[u8"code"].GetStringView(), pipeline, fd.Statements, { {}, scope });
			}
		}

		if (auto code = shaderJson[u8"code"]) {
			parseCode(code.GetStringView(), pipeline, pipeline.GetFunction({ {}, scope }, u8"main").Statements, {{}, scope});
		}

		return shader;
	}
};
