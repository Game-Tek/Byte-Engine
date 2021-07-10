#pragma once

#include <GAL/Pipelines.h>
#include <GAL/RenderCore.h>
#include <GTSL/Algorithm.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/File.h>
#include <GTSL/HashMap.h>
#include <GTSL/Serialize.h>
#include <GTSL/String.hpp>
#include <GTSL/Math/Vectors.h>

#include "ResourceManager.h"

#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Render/ShaderGenerator.h"

#include <GAL/Serialize.hpp>

class ShaderResourceManager final : public ResourceManager
{
	static GTSL::ShortString<12> ShaderTypeToFileExtension(GAL::ShaderType type)
	{
		switch (type)
		{
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
	ShaderResourceManager();
	~ShaderResourceManager();

	enum class ParameterType : uint8
	{
		UINT32, FVEC4,
		TEXTURE_REFERENCE, BUFFER_REFERENCE
	};
	
	struct Parameter
	{
		GTSL::Id64 Name;
		ParameterType Type;

		Parameter() = default;
		Parameter(const GTSL::Id64 name, const ParameterType type) : Name(name), Type(type) {}

		template<class ALLOC>
		friend void Insert(const Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(parameterInfo.Name, buffer);
			Insert(parameterInfo.Type, buffer);
		}
		
		template<class ALLOC>
		friend void Extract(Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(parameterInfo.Name, buffer);
			Extract(parameterInfo.Type, buffer);
		}
	};
	
	struct MaterialInstance
	{
		MaterialInstance() = default;
		
		union ParameterData
		{
			ParameterData() = default;
			
			uint32 uint32 = 0;
			GTSL::Vector4 Vector4;
			GTSL::Id64 TextureReference;
			uint64 BufferReference;

			template<class ALLOCATOR>
			friend void Insert(const ParameterData& uni, GTSL::Buffer<ALLOCATOR>& buffer) //if trivially copyable
			{
				buffer.CopyBytes(sizeof(ParameterData), reinterpret_cast<const byte*>(&uni));
			}

			template<class ALLOCATOR>
			friend void Extract(ParameterData& uni, GTSL::Buffer<ALLOCATOR>& buffer)
			{
				buffer.ReadBytes(sizeof(ParameterData), reinterpret_cast<byte*>(&uni));
			}
		};

		GTSL::ShortString<32> Name;
		GTSL::Array<GTSL::Pair<GTSL::Id64, ParameterData>, 16> Parameters;

		template<class ALLOC>
		friend void Insert(const MaterialInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(materialInstance.Name, buffer);
			Insert(materialInstance.Parameters, buffer);
		}

		template<class ALLOC>
		friend void Extract(MaterialInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(materialInstance.Name, buffer);
			Extract(materialInstance.Parameters, buffer);
		}

	};

	//struct VertexElement
	//
	//	GTSL::ShortString<32> VertexAttribute;
	//	GAL::ShaderDataType Type;
	//
	//	template<class ALLOC>
	//	friend void Insert(const VertexElement& vertexElement, GTSL::Buffer<ALLOC>& buffer)
	//	{
	//		Insert(vertexElement.VertexAttribute, buffer);
	//		Insert(vertexElement.Type, buffer);
	//	}
	//
	//	template<class ALLOC>
	//	friend void Extract(VertexElement& vertexElement, GTSL::Buffer<ALLOC>& buffer)
	//	{
	//		Extract(vertexElement.VertexAttribute, buffer);
	//		Extract(vertexElement.Type, buffer);
	//	}
	//{;
	
	struct VertexShader {
		GTSL::Array<GAL::Pipeline::VertexElement, 32> VertexElements;

		template<class ALLOC>
		friend void Insert(const VertexShader& vertex_shader, GTSL::Buffer<ALLOC>& buffer) {
			Insert(vertex_shader.VertexElements.GetLength(), buffer);
			for (uint32 i = 0; i < vertex_shader.VertexElements.GetLength(); ++i) {
				GAL::Insert(vertex_shader.VertexElements[i], buffer);
			}
		}

		template<class ALLOC>
		friend void Extract(VertexShader& vertex_shader, GTSL::Buffer<ALLOC>& buffer) {
			uint32 length = 0;
			Extract(length, buffer);

			for (uint32 i = 0; i < length; ++i) {
				GAL::Extract(vertex_shader.VertexElements.EmplaceBack(), buffer);
			}
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

		ShaderInfo()
		{
			
		}
		
		~ShaderInfo() {
			switch (Type)
			{
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
			default: ;
			}
		}
	};
	
	struct Shader : ShaderInfo {
		Shader() {
			
		}
		
		uint32 Size = 0, Offset = 0;

		template<class ALLOC>
		friend void Insert(const Shader& shader, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(shader.Name, buffer);
			Insert(shader.Type, buffer);
			Insert(shader.Size, buffer);
			Insert(shader.Offset, buffer);

			switch (shader.Type)
			{
			case GAL::ShaderType::VERTEX: Insert(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Insert(shader.FragmentShader, buffer); break;
			}
		}

		template<class ALLOC>
		friend void Extract(Shader& shader, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(shader.Name, buffer);
			Extract(shader.Type, buffer);
			Extract(shader.Size, buffer);
			Extract(shader.Offset, buffer);

			switch (shader.Type)
			{
			case GAL::ShaderType::VERTEX: Extract(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Extract(shader.FragmentShader, buffer); break;
			}
		}

		Shader& operator =(const Shader& other)
		{
			Offset = other.Offset;
			Size = other.Size;
			Name = other.Name;
			Type = other.Type;
			
			switch (Type)
			{
			case GAL::ShaderType::VERTEX: VertexShader = other.VertexShader; break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: FragmentShader = other.FragmentShader; break;
			case GAL::ShaderType::COMPUTE: break;
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
		
		~Shader() {
			switch (Type)
			{
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

	struct ShaderGroupData : Data
	{
		GTSL::ShortString<32> Name;
		GAL::ShaderStage Stages;
		GTSL::ShortString<32> RenderPass;
		GTSL::Array<GTSL::ShortString<32>, 16> Shaders;
	};
	
	struct ShaderGroupDataSerialize : DataSerialize<ShaderGroupData>
	{
		INSERT_START(ShaderGroupDataSerialize)
		{
			INSERT_BODY
			Insert(insertInfo.Name, buffer);
			Insert(insertInfo.Stages, buffer);
			Insert(insertInfo.RenderPass, buffer);
			Insert(insertInfo.Shaders, buffer);
		}

		EXTRACT_START(ShaderGroupDataSerialize)
		{
			EXTRACT_BODY
			Extract(extractInfo.Name, buffer);
			Extract(extractInfo.Stages, buffer);
			Extract(extractInfo.RenderPass, buffer);
			Extract(extractInfo.Shaders, buffer);
		}
	};

	struct ShaderGroupInfo
	{
		GTSL::ShortString<32> Name;
		GAL::ShaderStage Stages;
		GTSL::ShortString<32> RenderPass;
		GTSL::Array<Shader, 16> Shaders;
	};
	
	struct ShaderGroupCreateInfo
	{		
		GTSL::StaticString<64> Name;
		GTSL::Array<ShaderInfo, 16> Shaders;		
		GTSL::Array<Parameter, 16> Parameters;
		GTSL::Array<Parameter, 8> PerInstanceParameters;
		GTSL::Array<MaterialInstance, 16> MaterialInstances;
	};
	void CreateShaderGroup(const ShaderGroupCreateInfo& shader_group_create_info)
	{
		Id hashedName(shader_group_create_info.Name);		
		if (shaderGroups.Find(hashedName)) { return; }
		
		GTSL::Buffer<BE::TAR> shaderSourceBuffer; shaderSourceBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(8)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> indexBuffer; indexBuffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shaderBuffer; shaderBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(128)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shaderErrorBuffer; shaderErrorBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(4)), 8, GetTransientAllocator());

		ShaderGroupDataSerialize shader_group_data_serialize;
		shader_group_data_serialize.Name = shader_group_create_info.Name;
		shader_group_data_serialize.ByteOffset = shaderPackageFiles[0].GetSize();

		for (auto& s : shader_group_create_info.Shaders) {
			auto shaderTryEmplace = shaderInfos.TryEmplace(hashedName);

			if (!shaderTryEmplace) { continue; }

			GTSL::File shaderSourceFile;
			shaderSourceFile.Open(GetResourcePath(s.Name, ShaderTypeToFileExtension(s.Type)), GTSL::File::READ, false);

			GTSL::String string(8192, GetTransientAllocator());
			GenerateShader(string, s.Type);

			switch (s.Type) {
			case GAL::ShaderType::VERTEX:
				AddVertexShaderLayout(string, s.VertexShader.VertexElements);
				break;
			}

			shaderSourceFile.Read(shaderSourceBuffer.GetBufferInterface());

			//*shaderSourceBuffer.AllocateStructure<char8_t>() = '\0';
			string += GTSL::Range<const utf8*>(shaderSourceBuffer.GetLength(), reinterpret_cast<const utf8*>(shaderSourceBuffer.GetData()));
			
			const auto compilationResult = CompileShader(string, s.Name, s.Type, GAL::ShaderLanguage::GLSL, shaderBuffer.GetBufferInterface(), shaderErrorBuffer.GetBufferInterface());

			if (!compilationResult) {
				BE_LOG_MESSAGE(string)
				BE_LOG_ERROR(reinterpret_cast<const char*>(shaderErrorBuffer.GetData()));
			}

			shaderPackageFiles[0].Write(shaderBuffer);

			auto& shader = shaderTryEmplace.Get();
			shader.Name = s.Name;
			shader.Type = s.Type;
			shader.Size = shaderBuffer.GetLength();

			switch (s.Type)
			{
			case GAL::ShaderType::VERTEX: shader.VertexShader = s.VertexShader; break;
			case GAL::ShaderType::FRAGMENT: shader.FragmentShader = s.FragmentShader; break;
			case GAL::ShaderType::COMPUTE: break;
			case GAL::ShaderType::RAY_GEN: break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			default:;
			}

			shader_group_data_serialize.Shaders.EmplaceBack(s.Name);

			shaderSourceBuffer.Resize(0);
			shaderErrorBuffer.Resize(0);
			shaderBuffer.Resize(0);
		}

		shadersIndex.SetPointer(0);
		Insert(shaderGroups, indexBuffer);
		shadersIndex.Write(indexBuffer);
	}

	template<typename... ARGS>
	void LoadShaderGroupInfo(GameInstance* gameInstance, Id shaderGroupName, DynamicTaskHandle<ShaderResourceManager*, ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{		
		auto loadShaderGroup = [](TaskInfo taskInfo, ShaderResourceManager* materialResourceManager, Id shaderGroupName, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			ShaderGroupInfo shaderGroupInfo;

			for (auto& e : materialResourceManager->shaderGroups[shaderGroupName].Shaders) {
				auto& shader = materialResourceManager->shaderInfos[Id(e)];
				shaderGroupInfo.Shaders.EmplaceBack(shader);
			}

			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(materialResourceManager), GTSL::MoveRef(shaderGroupInfo), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask(u8"loadShaderInfosFromDisk", Task<ShaderResourceManager*, Id, decltype(dynamicTaskHandle), ARGS...>::Create(loadShaderGroup), GTSL::Range<TaskDependency*>(), this, GTSL::MoveRef(shaderGroupName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadShaderGroup(GameInstance* gameInstance, ShaderGroupInfo shader_group_info, DynamicTaskHandle<ShaderResourceManager*, ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, GTSL::Range<byte*> buffer, ARGS&&... args)
	{
		auto loadShaders = [](TaskInfo taskInfo, ShaderResourceManager* materialResourceManager, ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			uint32 offset = 0;

			for (auto& e : shader_group_info.Shaders) {
				auto& shader = materialResourceManager->shaderInfos[Id(e.Name)];
				
				materialResourceManager->shaderPackageFiles[materialResourceManager->getThread()].SetPointer(shader.Offset);

				[[maybe_unused]] const auto read = materialResourceManager->shaderPackageFiles[materialResourceManager->getThread()].Read(shader.Size, offset, buffer);

				offset += shader.Size;
			}

			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(materialResourceManager), GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask(u8"loadShadersFromDisk", Task<ShaderResourceManager*, ShaderGroupInfo, GTSL::Range<byte*>, decltype(dynamicTaskHandle), ARGS...>::Create(loadShaders), GTSL::Range<TaskDependency*>(), this, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File shaderGroupsIndex, shadersIndex;
	GTSL::HashMap<Id, ShaderGroupDataSerialize, BE::PersistentAllocatorReference> shaderGroups;
	GTSL::HashMap<Id, Shader, BE::PersistentAllocatorReference> shaderInfos;
	mutable GTSL::ReadWriteMutex mutex;

	GTSL::Array<GTSL::File, MAX_THREADS> shaderPackageFiles;
};
