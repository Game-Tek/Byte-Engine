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
		friend void Insert(const MaterialInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer) {
			Insert(materialInstance.Name, buffer);
			Insert(materialInstance.Parameters, buffer);
		}

		template<class ALLOC>
		friend void Extract(MaterialInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer) {
			Extract(materialInstance.Name, buffer);
			Extract(materialInstance.Parameters, buffer);
		}

	};
	
	struct VertexShader {
		GTSL::Array<GAL::Pipeline::VertexElement, 32> VertexElements;

		template<class ALLOC>
		friend void Insert(const VertexShader& vertex_shader, GTSL::Buffer<ALLOC>& buffer) {
			Insert(vertex_shader.VertexElements, buffer);
		}

		template<class ALLOC>
		friend void Extract(VertexShader& vertex_shader, GTSL::Buffer<ALLOC>& buffer) {
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

		ShaderInfo(const GTSL::ShortString<32>& string, GAL::ShaderType type) : Name(string), Type(type) {}
		
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

		Shader(const Shader& shader) : ShaderInfo(shader.Name, shader.Type), Size(shader.Size), Offset(shader.Offset)
		{
			switch (Type)
			{
			case GAL::ShaderType::VERTEX: VertexShader = shader.VertexShader; break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: FragmentShader = shader.FragmentShader; break;
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
		uint32 Size = 0;
		bool Valid = true;
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
			Insert(insertInfo.Size, buffer);
			Insert(insertInfo.Valid, buffer);
			Insert(insertInfo.RenderPass, buffer);
			Insert(insertInfo.Shaders, buffer);
		}

		EXTRACT_START(ShaderGroupDataSerialize)
		{
			EXTRACT_BODY
			Extract(extractInfo.Name, buffer);
			Extract(extractInfo.Stages, buffer);
			Extract(extractInfo.Size, buffer);
			Extract(extractInfo.Valid, buffer);
			Extract(extractInfo.RenderPass, buffer);
			Extract(extractInfo.Shaders, buffer);
		}
	};

	struct ShaderGroupInfo
	{
		GTSL::ShortString<32> Name;
		GAL::ShaderStage Stages;
		bool Valid = true;
		uint32 Size = 0;
		GTSL::ShortString<32> RenderPass;
		GTSL::Array<Shader, 16> Shaders;
	};
	
	struct ShaderGroupCreateInfo
	{		
		GTSL::StaticString<32> Name;
		GTSL::StaticString<32> RenderPass;
		GTSL::Array<ShaderInfo, 16> Shaders;		
		GTSL::Array<Parameter, 16> Parameters;
		GTSL::Array<Parameter, 8> PerInstanceParameters;
		GTSL::Array<MaterialInstance, 16> MaterialInstances;
	};
	void CreateShaderGroup(const ShaderGroupCreateInfo& shader_group_create_info)
	{
		Id hashedName(shader_group_create_info.Name);
		if (shaderGroupsMap.Find(hashedName)) { return; }
		
		GTSL::Buffer shaderSourceBuffer(GTSL::Byte(GTSL::KiloByte(8)), 8, GetTransientAllocator());
		GTSL::Buffer shaderBuffer(GTSL::Byte(GTSL::KiloByte(128)), 8, GetTransientAllocator());

		ShaderGroupDataSerialize& shaderGroupDataSerialize = shaderGroupsMap.Emplace(hashedName);
		shaderGroupDataSerialize.Name = shader_group_create_info.Name;
		shaderGroupDataSerialize.ByteOffset = 0xFFFFFFFF;
		shaderGroupDataSerialize.RenderPass = shader_group_create_info.RenderPass;
		
		for (auto& shaderCreateInfo : shader_group_create_info.Shaders) {
			auto shaderTryEmplace = shaderInfosMap.TryEmplace(Id(shaderCreateInfo.Name));
			if (!shaderTryEmplace) { continue; }

			GTSL::File shaderSourceFile;
			shaderSourceFile.Open(GetResourcePath(shaderCreateInfo.Name, ShaderTypeToFileExtension(shaderCreateInfo.Type)), GTSL::File::READ, false);

			GTSL::String shaderCode(8192, GetTransientAllocator());
			GenerateShader(shaderCode, shaderCreateInfo.Type);

			switch (shaderCreateInfo.Type) {
			case GAL::ShaderType::VERTEX:
				AddVertexShaderLayout(shaderCode, shaderCreateInfo.VertexShader.VertexElements);
				break;
			}

			shaderSourceFile.Read(shaderSourceBuffer);

			shaderCode += GTSL::Range<const utf8*>(shaderSourceBuffer.GetLength(), reinterpret_cast<const utf8*>(shaderSourceBuffer.GetData()));

			//DON'T push null terminator, glslang doesn't like it
			
			GTSL::String compilationErrorString(8192, GetTransientAllocator());
			const auto compilationResult = CompileShader(shaderCode, shaderCreateInfo.Name, shaderCreateInfo.Type, GAL::ShaderLanguage::GLSL, shaderBuffer, compilationErrorString);

			if (!compilationResult) {
				BE_LOG_ERROR(compilationErrorString);
			}

			
			auto& shader = shaderTryEmplace.Get();
			shader.Name = shaderCreateInfo.Name;
			shader.Type = shaderCreateInfo.Type;
			shader.Size = shaderBuffer.GetLength();
			shader.Offset = shaderPackageFiles[0].GetSize();

			switch (shaderCreateInfo.Type)
			{
			case GAL::ShaderType::VERTEX: shader.VertexShader = shaderCreateInfo.VertexShader; shaderGroupDataSerialize.Stages |= GAL::ShaderStages::VERTEX; break;
			case GAL::ShaderType::FRAGMENT: shader.FragmentShader = shaderCreateInfo.FragmentShader; shaderGroupDataSerialize.Stages |= GAL::ShaderStages::FRAGMENT; break;
			case GAL::ShaderType::COMPUTE: break;
			case GAL::ShaderType::RAY_GEN: break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			}

			shaderGroupDataSerialize.Size += shaderBuffer.GetLength();
			shaderGroupDataSerialize.Shaders.EmplaceBack(shaderCreateInfo.Name);

			if (!shaderBuffer.GetLength()) {
				shaderGroupDataSerialize.Valid = false;
			}
			
			shaderPackageFiles[0].Write(shaderBuffer);

			shaderSourceBuffer.Resize(0);
			shaderBuffer.Resize(0);
		}
		
		{
			GTSL::Buffer fileBuffer(GetTransientAllocator());
			
			shaderGroupsInfoFile.SetPointer(0);
			Insert(shaderGroupsMap, fileBuffer);
			shaderGroupsInfoFile.Write(fileBuffer);
		}

		{
			GTSL::Buffer fileBuffer(GetTransientAllocator());
			
			shaderInfosFile.SetPointer(0);
			Insert(shaderInfosMap, fileBuffer);
			shaderInfosFile.Write(fileBuffer);
		}
	}

	template<typename... ARGS>
	void LoadShaderGroupInfo(GameInstance* gameInstance, Id shaderGroupName, DynamicTaskHandle<ShaderResourceManager*, ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{		
		auto loadShaderGroup = [](TaskInfo taskInfo, ShaderResourceManager* materialResourceManager, Id shaderGroupName, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			auto& shaderGroup = materialResourceManager->shaderGroupsMap[shaderGroupName];
			
			ShaderGroupInfo shaderGroupInfo;

			shaderGroupInfo.Name = shaderGroup.Name;
			shaderGroupInfo.Size = shaderGroup.Size;
			shaderGroupInfo.Valid = shaderGroup.Valid;
			shaderGroupInfo.RenderPass = shaderGroup.RenderPass;
			shaderGroupInfo.Stages = shaderGroup.Stages;
			
			for (auto& e : materialResourceManager->shaderGroupsMap[shaderGroupName].Shaders) {
				auto& shader = materialResourceManager->shaderInfosMap[Id(e)];
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
				materialResourceManager->shaderPackageFiles[materialResourceManager->getThread()].SetPointer(e.Offset);
				[[maybe_unused]] const auto read = materialResourceManager->shaderPackageFiles[materialResourceManager->getThread()].Read(e.Size, offset, buffer);
				offset += e.Size;
			}

			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(materialResourceManager), GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask(u8"loadShadersFromDisk", Task<ShaderResourceManager*, ShaderGroupInfo, GTSL::Range<byte*>, decltype(dynamicTaskHandle), ARGS...>::Create(loadShaders), GTSL::Range<TaskDependency*>(), this, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File shaderGroupsInfoFile, shaderInfosFile;
	GTSL::HashMap<Id, ShaderGroupDataSerialize, BE::PersistentAllocatorReference> shaderGroupsMap;
	GTSL::HashMap<Id, Shader, BE::PersistentAllocatorReference> shaderInfosMap;
	mutable GTSL::ReadWriteMutex mutex;

	GTSL::Array<GTSL::File, MAX_THREADS> shaderPackageFiles;
};
