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
#include <GTSL/Math/Vectors.h>

#include <GAL/Serialize.hpp>

#include "ResourceManager.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Render/ShaderGenerator.h"

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
	ShaderResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"ShaderResourceManager"), shaderGroupsMap(8, GetPersistentAllocator()), shaderInfosMap(8, GetPersistentAllocator())
	{
		shaderPackageFile.Open(GetResourcePath(GTSL::ShortString<32>(u8"Shaders"), GTSL::ShortString<32>(u8"bepkg")), 1 * 1024 * 1024, GTSL::File::READ | GTSL::File::WRITE);
		
		switch (shaderInfosFile.Open(GetResourcePath(GTSL::ShortString<32>(u8"Shaders"), GTSL::ShortString<32>(u8"beidx")), GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		}

		switch (shaderGroupsInfoFile.Open(GetResourcePath(GTSL::ShortString<32>(u8"ShaderGroups"), GTSL::ShortString<32>(u8"beidx")), GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		}

		{
			GTSL::Buffer fileBuffer(GetTransientAllocator());

			shaderGroupsInfoFile.Read(fileBuffer);

			if (fileBuffer.GetLength()) {
				Extract(shaderGroupsMap, fileBuffer);
			}
		}

		{
			GTSL::Buffer fileBuffer(GetTransientAllocator());

			shaderInfosFile.Read(fileBuffer);

			if (fileBuffer.GetLength()) {
				Extract(shaderInfosMap, fileBuffer);
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
			Insert(parameterInfo.Name, buffer);
			Insert(parameterInfo.Type, buffer);
			Insert(parameterInfo.Value, buffer);
		}
		
		template<class ALLOC>
		friend void Extract(Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer) {
			Extract(parameterInfo.Name, buffer);
			Extract(parameterInfo.Type, buffer);
			Extract(parameterInfo.Value, buffer);
		}
	};
	
	struct ShaderGroupInstance
	{
		ShaderGroupInstance() = default;

		GTSL::ShortString<32> Name;
		GTSL::StaticVector<Parameter, 16> Parameters;

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

		ShaderInfo(const GTSL::ShortString<32>& string, GAL::ShaderType type) : Name(string), Type(type), Parameters(1) {
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
			default: ;
			}
		}
	};
	
	struct Shader : ShaderInfo {		
		uint32 Size = 0, Offset = 0;

		template<class ALLOC>
		friend void Insert(const Shader& shader, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(shader.Name, buffer);
			Insert(shader.Type, buffer);
			Insert(shader.Size, buffer);
			Insert(shader.Offset, buffer);
			Insert(shader.Parameters, buffer);

			switch (shader.Type) {
			case GAL::ShaderType::VERTEX: Insert(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Insert(shader.FragmentShader, buffer); break;
			case GAL::ShaderType::COMPUTE: Insert(shader.ComputeShader, buffer); break;
			}
		}

		template<class ALLOC>
		friend void Extract(Shader& shader, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(shader.Name, buffer);
			Extract(shader.Type, buffer);
			Extract(shader.Size, buffer);
			Extract(shader.Offset, buffer);
			Extract(shader.Parameters, buffer);

			switch (shader.Type) {
			case GAL::ShaderType::VERTEX: Extract(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Extract(shader.FragmentShader, buffer); break;
			case GAL::ShaderType::COMPUTE: Extract(shader.ComputeShader, buffer); break;
			}
		}

		Shader(const GTSL::ShortString<32> name, const GAL::ShaderType type) : ShaderInfo(name, type) {

		}
		
		Shader(const Shader& shader) : ShaderInfo(shader), Size(shader.Size), Offset(shader.Offset) {
		}
		
		Shader& operator=(const Shader& other)
		{
			Offset = other.Offset;
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

	struct ShaderGroupData : Data
	{
		GTSL::ShortString<32> Name;
		GAL::ShaderStage Stages;
		uint32 Size = 0;
		bool Valid = true;
		GTSL::ShortString<32> RenderPass;
		GTSL::StaticVector<GTSL::ShortString<32>, 16> Shaders;
		GTSL::StaticVector<ShaderGroupInstance, 16> Instances;
		GTSL::StaticVector<Parameter, 16> Parameters;
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
			Insert(insertInfo.Instances, buffer);
			Insert(insertInfo.Parameters, buffer);
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
			Extract(extractInfo.Instances, buffer);
			Extract(extractInfo.Parameters, buffer);
		}
	};

	struct ShaderGroupInfo {
		GTSL::ShortString<32> Name;
		GAL::ShaderStage Stages;
		bool Valid = true;
		uint32 Size = 0;
		GTSL::ShortString<32> RenderPass;
		GTSL::StaticVector<Shader, 16> Shaders;
		GTSL::StaticVector<ShaderGroupInstance, 16> Instances;
		GTSL::StaticVector<Parameter, 16> Parameters;
	};
	
	struct ShaderGroupCreateInfo {
		GTSL::StaticString<32> Name;
		GTSL::StaticString<32> RenderPass;
		GTSL::StaticVector<::Shader*, 16> Shaders;
		GTSL::StaticVector<Parameter, 16> Parameters;
		GTSL::StaticVector<Parameter, 8> PerInstanceParameters;
		GTSL::StaticVector<ShaderGroupInstance, 16> MaterialInstances;
	};
	void CreateShaderGroup(const ShaderGroupCreateInfo& shader_group_create_info) {
		Id hashedName(shader_group_create_info.Name);
		if (shaderGroupsMap.Find(hashedName)) { return; }

		GTSL::Buffer shaderBuffer(GTSL::Byte(GTSL::KiloByte(128)), 8, GetTransientAllocator());

		ShaderGroupDataSerialize& shaderGroupDataSerialize = shaderGroupsMap.Emplace(hashedName);
		shaderGroupDataSerialize.Name = shader_group_create_info.Name;
		shaderGroupDataSerialize.ByteOffset = 0xFFFFFFFF;
		shaderGroupDataSerialize.RenderPass = shader_group_create_info.RenderPass;
		shaderGroupDataSerialize.Instances = shader_group_create_info.MaterialInstances;
		
		for (auto& shaderCreateInfo : shader_group_create_info.Shaders) {
			auto shaderTryEmplace = shaderInfosMap.TryEmplace(Id(shaderCreateInfo->Name), shaderCreateInfo->Name, shaderCreateInfo->TargetSemantics);
			if (!shaderTryEmplace) { continue; }

			for (auto& e : shaderCreateInfo->ShaderParameters) {
				shaderGroupDataSerialize.Parameters.EmplaceBack(e.First, e.Second, u8"");
			}
			
			auto shaderCode = GenerateShader(*shaderCreateInfo);
			
			GTSL::String compilationErrorString(8192, GetTransientAllocator());

			const auto compilationResult = CompileShader(shaderCode, shaderCreateInfo->Name, shaderCreateInfo->TargetSemantics, GAL::ShaderLanguage::GLSL, shaderBuffer, compilationErrorString);

			if (!compilationResult) {
				BE_LOG_ERROR(compilationErrorString);
			}
			
			auto& shader = shaderTryEmplace.Get();
			shader.Size = shaderBuffer.GetLength();
			shader.Offset = size;

			if (!shaderBuffer.GetLength()) {
				shaderGroupDataSerialize.Valid = false;
			}
			
			GTSL::MemCopy(shader.Size, shaderBuffer.begin(), shaderPackageFile.GetData() + size);

			switch (shaderCreateInfo->TargetSemantics) {
			case GAL::ShaderType::VERTEX: shader.VertexShader.VertexElements = shaderCreateInfo->VertexElements; shaderGroupDataSerialize.Stages |= GAL::ShaderStages::VERTEX; break;
			case GAL::ShaderType::FRAGMENT: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::FRAGMENT; break;
			case GAL::ShaderType::COMPUTE: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::COMPUTE; break;
			case GAL::ShaderType::RAY_GEN: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::RAY_GEN; break;
			case GAL::ShaderType::ANY_HIT: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::ANY_HIT; break;
			case GAL::ShaderType::CLOSEST_HIT: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::CLOSEST_HIT; break;
			case GAL::ShaderType::MISS: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::MISS; break;
			case GAL::ShaderType::INTERSECTION: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::INTERSECTION; break;
			case GAL::ShaderType::CALLABLE: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::CALLABLE; break;
			case GAL::ShaderType::TASK: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::TASK; break;
			case GAL::ShaderType::MESH: shaderGroupDataSerialize.Stages |= GAL::ShaderStages::MESH; break;
			}

			size += shaderBuffer.GetLength();
			shaderGroupDataSerialize.Size += shaderBuffer.GetLength();
			shaderGroupDataSerialize.Shaders.EmplaceBack(shaderCreateInfo->Name);
			
			shaderPackageFile.Resize(shaderGroupDataSerialize.Size);

			shaderBuffer.Clear();
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
	void LoadShaderGroupInfo(ApplicationManager* gameInstance, Id shaderGroupName, DynamicTaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"loadShaderInfosFromDisk", {}, &ShaderResourceManager::loadShaderGroup<ARGS...>, {}, {}, GTSL::MoveRef(shaderGroupName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
	template<typename... ARGS>
	void LoadShaderGroup(ApplicationManager* gameInstance, ShaderGroupInfo shader_group_info, DynamicTaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, GTSL::Range<byte*> buffer, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"loadShadersFromDisk", {}, &ShaderResourceManager::loadShaders<ARGS...>, {}, {}, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File shaderGroupsInfoFile, shaderInfosFile;
	GTSL::HashMap<Id, ShaderGroupDataSerialize, BE::PersistentAllocatorReference> shaderGroupsMap;
	GTSL::HashMap<Id, Shader, BE::PersistentAllocatorReference> shaderInfosMap;
	mutable GTSL::ReadWriteMutex mutex;

	GTSL::MappedFile shaderPackageFile;

	uint64 size = 0;

	template<typename... ARGS>
	void loadShaderGroup(TaskInfo taskInfo, Id shaderGroupName, DynamicTaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS... args) { //TODO: check why can't use ARGS&&
		auto& shaderGroup = shaderGroupsMap[shaderGroupName];

		ShaderGroupInfo shaderGroupInfo;

		shaderGroupInfo.Name = shaderGroup.Name;
		shaderGroupInfo.Size = shaderGroup.Size;
		shaderGroupInfo.Valid = shaderGroup.Valid;
		shaderGroupInfo.RenderPass = shaderGroup.RenderPass;
		shaderGroupInfo.Stages = shaderGroup.Stages;
		shaderGroupInfo.Instances = shaderGroup.Instances;
		shaderGroupInfo.Parameters = shaderGroup.Parameters;

		for (auto& e : shaderGroupsMap[shaderGroupName].Shaders) {
			const auto& shader = shaderInfosMap[Id(e)];
			shaderGroupInfo.Shaders.EmplaceBack(shader);
		}

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(shaderGroupInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadShaders(TaskInfo taskInfo, ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer, DynamicTaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, ARGS... args) {
		uint32 offset = 0;

		for (auto& e : shader_group_info.Shaders) {
			GTSL::MemCopy(e.Size, shaderPackageFile.GetData() + e.Offset, buffer.begin() + offset);
			offset += e.Size;
		}

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
	};
};
