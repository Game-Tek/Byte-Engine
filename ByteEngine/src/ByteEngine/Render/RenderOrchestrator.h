#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Array.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/FunctionPointer.hpp>

#include "ByteEngine/Id.h"
#include <GTSL/Vector.hpp>

#include "MaterialSystem.h"
#include "RenderSystem.h"
#include "RenderTypes.h"
#include "ByteEngine/Game/Tasks.h"

class RenderOrchestrator;
class RenderState;
class MaterialSystem;
class RenderSystem;
class RenderGroup;
struct TaskInfo;

class RenderManager : public System
{
public:
	virtual void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) = 0;

	struct SetupInfo
	{
		GameInstance* GameInstance;
		RenderSystem* RenderSystem;
		MaterialSystem* MaterialSystem;
		//RenderState* RenderState;
		GTSL::Matrix4 ViewMatrix, ProjectionMatrix;
		RenderOrchestrator* RenderOrchestrator;
	};
	virtual void Setup(const SetupInfo& info) = 0;
};

class StaticMeshRenderManager : public RenderManager
{
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;

private:	
	MemberHandle<void*> staticMeshStruct;
	MemberHandle<GTSL::Matrix4> matrixUniformBufferMemberHandle;
	MemberHandle<RenderSystem::BufferAddress> vertexBufferReferenceHandle, indexBufferReferenceHandle;
};

class UIRenderManager : public RenderManager
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;
	RenderSystem::MeshHandle GetSquareMesh() const { return square; }
	MaterialInstanceHandle GetUIMaterial() const { return uiMaterial; }

private:
	RenderSystem::MeshHandle square;

	MemberHandle<GTSL::Matrix4> matrixUniformBufferMemberHandle, colorHandle;

	uint8 comps = 2;
	MaterialInstanceHandle uiMaterial;
};

class RenderOrchestrator : public System
{
public:
	RenderOrchestrator() : System("RenderOrchestrator") {}
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo);

	void AddRenderManager(GameInstance* gameInstance, const Id renderManager, const SystemHandle systemReference);
	void RemoveRenderManager(GameInstance* gameInstance, const Id renderGroupName, const SystemHandle systemReference);

	struct CreateMaterialInfo
	{
		Id MaterialName;
		MaterialResourceManager* MaterialResourceManager = nullptr;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager;
	};
	[[nodiscard]] MaterialInstanceHandle CreateMaterial(const CreateMaterialInfo& info);
	[[nodiscard]] MaterialInstanceHandle CreateRayTracingMaterial(const CreateMaterialInfo& info);

	MaterialInstanceHandle GetMaterialHandle(Id name) { return name; }
	
	GTSL::uint8 GetRenderPassColorWriteAttachmentCount(const Id renderPassName)
	{
		auto& renderPass = renderPassesMap[renderPassName];
		uint8 count = 0;
		for(const auto& e : renderPass.WriteAttachments) {
			if (e.Layout == TextureLayout::COLOR_ATTACHMENT || e.Layout == TextureLayout::GENERAL) { ++count; }
		}
		return count;
	}

	void AddSetToRenderGroup(Id renderGroupName, Id setName) {
		renderGroups.At(renderGroupName).Sets.EmplaceBack(setName);
	}

	void AddAttachment(Id name, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, TextureType::value_type type, GTSL::RGBA clearColor);

	enum class PassType : uint8
	{
		RASTER, COMPUTE, RAY_TRACING
	};

	struct AttachmentInfo
	{
		Id Name;
		TextureLayout StartState, EndState;
		GAL::RenderTargetLoadOperations Load;
		GAL::RenderTargetStoreOperations Store;
	};
	
	struct PassData
	{
		Id Name;
		
		struct AttachmentReference {
			Id Name;
		};
		GTSL::Array<AttachmentReference, 8> ReadAttachments, WriteAttachments;

		PassType PassType;

		Id ResultAttachment;
	};
	void AddPass(RenderSystem* renderSystem, MaterialSystem* materialSystem, GTSL::Range<const PassData*> passesData);

	void OnResize(RenderSystem* renderSystem, MaterialSystem* materialSystem, const GTSL::Extent2D newSize);

	/**
	 * \brief Enables or disables the rendering of a render pass
	 * \param renderPassName Name of the render Pass to toggle
	 * \param enable Whether to enable(true) or disable(false) the render pass
	 */
	void ToggleRenderPass(Id renderPassName, bool enable);

	void AddToRenderPass(Id renderPass, Id renderGroup);

	void AddMesh(const RenderSystem::MeshHandle meshHandle, const MaterialInstanceHandle materialHandle, const uint32 instanceIndex);

	MAKE_HANDLE(uint8, IndexStream)
	
	IndexStreamHandle AddIndexStream() { return IndexStreamHandle(renderState.IndexStreams.EmplaceBack(0)); }
	void UpdateIndexStream(IndexStreamHandle indexStreamHandle, CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem);
	void UpdateIndexStream(IndexStreamHandle indexStreamHandle, CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 value);
	void PopIndexStream(IndexStreamHandle indexStreamHandle) { renderState.IndexStreams[indexStreamHandle()] = 0; renderState.IndexStreams.PopBack(); }

	void BindData(const RenderSystem* renderSystem, const MaterialSystem* materialSystem, CommandBuffer commandBuffer, Buffer buffer);

	void PopData() { renderState.Offset -= 4; }

	void BindMaterial(RenderSystem* renderSystem, CommandBuffer commandBuffer, MaterialHandle materialHandle);

	void OnRenderEnable(TaskInfo taskInfo, bool oldFocus);
	void OnRenderDisable(TaskInfo taskInfo, bool oldFocus);

private:
	inline static const Id RENDER_TASK_NAME{ "RenderRenderGroups" };
	inline static const Id SETUP_TASK_NAME{ "SetupRenderGroups" };
	inline static const Id CLASS_NAME{ "RenderOrchestrator" };

	void onRenderEnable(GameInstance* gameInstance, const GTSL::Range<const TaskDependency*> dependencies);
	void onRenderDisable(GameInstance* gameInstance);

	bool renderingEnabled = false;
	
	SubSetHandle renderGroupsSubSet;
	SubSetHandle renderPassesSubSet;

	MemberHandle<GTSL::Matrix4> cameraMatricesHandle;
	BufferHandle cameraDataBuffer;
	BufferHandle globalDataBuffer;
	MemberHandle<uint32> globalDataHandle;
	SubSetHandle textureSubsetsHandle;
	SubSetHandle imagesSubsetHandle;
	SubSetHandle topLevelAsHandle;

	struct RenderGroupData
	{
		uint32 Index;
		GTSL::Array<uint8, 8> IndexStreams;
		GTSL::Array<Id, 8> Sets;
	};
	GTSL::StaticMap<Id, RenderGroupData, 16> renderGroups;
	
	struct RenderState
	{
		Id BoundRenderGroup;
		GTSL::Array<uint32, 8> IndexStreams; // MUST be 4 bytes or push constant reads will be messed up
		//PipelineLayout PipelineLayout;
		PipelineType PipelineType;
		ShaderStage::value_type ShaderStages = ShaderStage::ALL;
		uint8 Offset = 0;
		PipelineLayout PipelineLayout;
	} renderState;
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 32>, BE::PersistentAllocatorReference> setupSystemsAccesses;
	
	GTSL::FlatHashMap<Id, SystemHandle, BE::PersistentAllocatorReference> renderManagers;

	struct ExecuteCommand
	{
		union
		{
			GTSL::Extent3D LaunchMatrix;

			union
			{
				RenderSystem::MeshHandle MeshHandle; uint32 InstanceCount;
			};
		};
	};
	
	Id resultAttachment;
	
	GTSL::Array<Id, 8> renderPasses;

	struct AttachmentData
	{
		Id Name;
		TextureLayout Layout;
		PipelineStage::value_type ConsumingStages;
	};
	
	struct RenderPassData
	{
		bool Enabled = false;
		uint8 APIRenderPass = 0;
		
		GTSL::Array<Id, 8> RenderGroups;
		PassType PassType;
		GTSL::Array<AttachmentData, 8> WriteAttachments, ReadAttachments;
		
		PipelineStage::value_type PipelineStages;
		SetHandle AttachmentsSetHandle;
		MemberHandle<uint32> AttachmentsIndicesHandle;
		BufferHandle BufferHandle;
	};
	GTSL::FlatHashMap<Id, RenderPassData, BE::PAR> renderPassesMap;
	GTSL::Array<Id, 8> renderPassesNames;

	AccessFlags::value_type accessFlagsFromStageAndAccessType(PipelineStage::value_type, bool writeAccess);
	
	using RenderPassFunctionType = GTSL::FunctionPointer<void(GameInstance*, RenderSystem*, MaterialSystem*, CommandBuffer, Id)>;
	
	GTSL::StaticMap<Id, RenderPassFunctionType, 8> renderPassesFunctions;

	void renderScene(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);
	void renderUI(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);
	void renderRays(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);
	void dispatch(GameInstance* gameInstance, RenderSystem* renderSystem, MaterialSystem* materialSystem,
	              CommandBuffer commandBuffer, Id rp);

	void transitionImages(CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, Id renderPassId);

	struct ShaderLoadInfo
	{
		ShaderLoadInfo() = default;
		ShaderLoadInfo(ShaderLoadInfo&& other) noexcept : Buffer(GTSL::MoveRef(other.Buffer)), Component(other.Component) {}
		GTSL::Buffer<BE::PAR> Buffer; uint32 Component;
	};

	void onShaderInfosLoaded(TaskInfo taskInfo, MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaderInfos, ShaderLoadInfo shaderLoadInfo);
	void onShadersLoaded(TaskInfo taskInfo, MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaders, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo);

	void traceRays(GTSL::Extent2D rayGrid, CommandBuffer* commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem);
	
	//MATERIAL STUFF
	struct RayTracingPipelineData
	{
		struct ShaderGroupData
		{
			uint32 Count = 0, Size = 0;
			GAL::VulkanDeviceAddress Address;
			BufferHandle Buffer;

			struct ShaderRegisterData
			{
				GTSL::Array<Id, 8> Buffers;
				MemberHandle<uint32> ShaderHandle;
				MemberHandle<uint32> BufferBufferReferencesMemberHandle;
			};
			
			GTSL::Vector<ShaderRegisterData, BE::PAR> Shaders;
		} ShaderGroups[4];

		RayTracingPipeline Pipeline;
	};
	GTSL::KeepVector<RayTracingPipelineData, BE::PAR> rayTracingPipelines;
	
	struct PrivateMaterialHandle
	{
		uint32 MaterialIndex = 0, MaterialInstance = 0, SubMaterialIndex = 0;
	};

	uint32 textureIndex = 0, imageIndex = 0;
	
	struct CreateTextureInfo
	{
		Id TextureName;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager = nullptr;
		PrivateMaterialHandle MaterialHandle;
	};
	uint32 createTexture(const CreateTextureInfo& createTextureInfo);
	
	struct MaterialLoadInfo
	{
		MaterialLoadInfo(RenderSystem* renderSystem, GTSL::Buffer<BE::PAR>&& buffer, uint32 index, uint32 instanceIndex, TextureResourceManager* tRM) : RenderSystem(renderSystem), Buffer(MoveRef(buffer)), Component(index), InstanceIndex(instanceIndex), TextureResourceManager(tRM)
		{

		}

		RenderSystem* RenderSystem = nullptr;
		GTSL::Buffer<BE::PAR> Buffer;
		uint32 Component, InstanceIndex;
		TextureResourceManager* TextureResourceManager;
	};
	void onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo);
	
	struct MaterialData
	{
		MaterialHandle Name;

		GTSL::Vector<GTSL::Pair<uint32, uint32>, BE::PAR> MaterialInstances;

		RasterizationPipeline Pipeline;
		Id RenderGroup;
		uint32 InstanceCount = 0;

		GTSL::StaticMap<Id, MemberHandle<uint32>, 16> ParametersHandles;

		GTSL::Array<MaterialResourceManager::Parameter, 16> Parameters;
		MemberHandle<void*> MaterialInstancesMemberHandle;
	};
	GTSL::KeepVector<MaterialData, BE::PAR> materials;
	GTSL::FlatHashMap<Id, uint32, BE::PAR> loadedMaterials;

	struct MaterialInstanceData
	{
		MaterialInstanceHandle Name;

		uint32 Material = 0;
		uint8 Counter = 0, Target = 0;

		struct MeshData
		{
			RenderSystem::MeshHandle Handle;
			uint32 InstanceCount = 0, InstanceIndex = 0;
		};
		
		GTSL::Vector<MeshData, BE::PAR> Meshes;
	};
	GTSL::KeepVector<MaterialInstanceData, BE::PAR> materialInstances;
	GTSL::FlatHashMap<Id, uint32, BE::PAR> loadedMaterialInstances;
	GTSL::FlatHashMap<Id, MaterialInstanceData, BE::PAR> awaitingMaterialInstances;
	GTSL::FlatHashMap<Id, uint32, BE::PAR> materialInstancesByName;

	struct TextureLoadInfo
	{
		TextureLoadInfo() = default;

		TextureLoadInfo(uint32 component, RenderSystem* renderSystem, RenderAllocation renderAllocation) : Component(component), RenderSystem(renderSystem), RenderAllocation(renderAllocation)
		{
		}

		uint32 Component;
		RenderSystem* RenderSystem;
		RenderAllocation RenderAllocation;
		RenderSystem::TextureHandle TextureHandle;
	};
	void onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);
	void onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);
	
	//MATERIAL STUFF

	GTSL::FlatHashMap<Id, uint32, BE::PersistentAllocatorReference> texturesRefTable;

	GTSL::Vector<uint32, BE::PAR> latestLoadedTextures;
	GTSL::KeepVector<GTSL::Vector<PrivateMaterialHandle, BE::PAR>, BE::PersistentAllocatorReference> pendingMaterialsPerTexture;

	void setMaterialInstanceAsLoaded(const PrivateMaterialHandle privateMaterialHandle, const MaterialInstanceHandle materialInstanceHandle);
	
	void addPendingMaterialToTexture(uint32 texture, PrivateMaterialHandle material) {
		pendingMaterialsPerTexture[texture].EmplaceBack(material);
	}

	struct APIRenderPassData
	{
		Id Name;
		RenderPass RenderPass;
		FrameBuffer FrameBuffer;
		GTSL::Array<Id, 16> UsedAttachments;
	};
	GTSL::Array<APIRenderPassData, 16> apiRenderPasses;

	struct SubPass
	{
		Id Name;
		uint8 DepthAttachment;
	};
	GTSL::Array<GTSL::Array<SubPass, 16>, 16> subPasses;

	struct Attachment
	{
		RenderSystem::TextureHandle TextureHandle;

		Id Name;
		TextureType::value_type Type;
		TextureUses Uses;
		TextureLayout Layout;
		PipelineStage::value_type ConsumingStages;
		bool WriteAccess = false;
		GTSL::RGBA ClearColor;
		GAL::FormatDescriptor FormatDescriptor;
		uint32 ImageIndex;
	};
	GTSL::StaticMap<Id, Attachment, 32> attachments;

	void updateImage(Attachment& attachment, TextureLayout textureLayout, PipelineStage::value_type stages, bool writeAccess)
	{
		attachment.Layout = textureLayout; attachment.ConsumingStages = stages; attachment.WriteAccess = writeAccess;
	}

	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, ::RenderOrchestrator::TextureLoadInfo> onTextureInfoLoadHandle;
	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, ::RenderOrchestrator::TextureLoadInfo> onTextureLoadHandle;
	DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, ShaderLoadInfo> onShaderInfosLoadHandle;
	DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, GTSL::Range<byte*>, ShaderLoadInfo> onShadersLoadHandle;

	[[nodiscard]] RenderPass getAPIRenderPass(const Id renderPassName) const { return apiRenderPasses[renderPassesMap.At(renderPassName).APIRenderPass].RenderPass; }
	[[nodiscard]] uint8 getAPISubPassIndex(const Id renderPass) const {
		uint8 i = 0;
		for (auto& e : subPasses[renderPassesMap.At(renderPass).APIRenderPass]) { if (e.Name == renderPass) { return i; } }
	}
	[[nodiscard]] FrameBuffer getFrameBuffer(const uint8 rp) const { return apiRenderPasses[rp].FrameBuffer; }
};
