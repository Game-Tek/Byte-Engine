#pragma once

#include <any>

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
	MemberHandle<uint32> materialInstance;
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
	MemberHandle<void*> uiDataStruct;

	uint8 comps = 2;
	MaterialInstanceHandle uiMaterial;
};

class RenderOrchestrator : public System
{
public:
	enum class PassType : uint8 {
		RASTER, COMPUTE, RAY_TRACING
	};
protected:
	enum class LayerType {
		DISPATCH, RAY_TRACE, MATERIAL, MESHES, RENDER_PASS, LAYER
	};

	enum class InternalLayerType {
		DISPATCH, RAY_TRACE, MATERIAL_AND_MESHES, RENDER_PASS, LAYER
	};
	
	struct Layer {
		LayerType Type;
		InternalLayerType PrivateType;
		Id Name; uint32 Index; bool Indexed;

		Layer() {
		}
		
		struct MeshInstanceData {
			RenderSystem::MeshHandle Handle;
			uint32 InstanceCount;
		};

		struct MaterialData {
			MaterialInstanceHandle MaterialHandle;

			struct VertexGroup {
				GTSL::Array<MeshInstanceData, 16> Meshes;
				uint8 VertexGroupIndex;
			};

			GTSL::Array<VertexGroup, 8> MeshInstancesPerVertexGroup;
		};

		struct DispatchData {
			GTSL::Extent3D DispatchSize;
		};

		struct RayTraceData {
			GTSL::Extent3D DispatchSize;
		};

		struct AttachmentData {
			Id Name;
			GAL::TextureLayout Layout;
			GAL::PipelineStage ConsumingStages;
		};

		struct RenderPassData {
			bool Enabled = false;
			uint8 APIRenderPass = 0;

			PassType PassType;
			GTSL::Array<AttachmentData, 8> WriteAttachments, ReadAttachments;

			GAL::PipelineStage PipelineStages;
			SetHandle AttachmentsSetHandle;
			MemberHandle<uint32> AttachmentsIndicesHandle;
			BufferHandle BufferHandle;
			Id Name;
		};
		
		union {
			MaterialData Material;
			DispatchData Dispatch;
			RayTraceData RayTrace;
			RenderPassData RenderPass;
		};

		~Layer() {
			switch (PrivateType) {
			case InternalLayerType::DISPATCH: Dispatch.~DispatchData();  break;
			case InternalLayerType::RAY_TRACE: RayTrace.~RayTraceData(); break;
			case InternalLayerType::MATERIAL_AND_MESHES: Material.~MaterialData(); break;
			case InternalLayerType::RENDER_PASS: RenderPass.~RenderPassData(); break;
			case InternalLayerType::LAYER: break;
			default:;
			}
		}
	};
	
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
		Id MaterialName, InstanceName;
		MaterialResourceManager* MaterialResourceManager = nullptr;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager;
	};
	[[nodiscard]] MaterialInstanceHandle CreateMaterial(const CreateMaterialInfo& info);
	[[nodiscard]] MaterialInstanceHandle CreateRayTracingMaterial(const CreateMaterialInfo& info);
	
	GTSL::uint8 GetRenderPassColorWriteAttachmentCount(const Id renderPassName) {
		auto& renderPass = renderingTreeMap[renderPassName]->Data.RenderPass;
		uint8 count = 0;
		for(const auto& e : renderPass.WriteAttachments) {
			if (e.Layout == GAL::TextureLayout::ATTACHMENT || e.Layout == GAL::TextureLayout::GENERAL) { ++count; }
		}
		return count;
	}

	void AddAttachment(Id name, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type, GTSL::RGBA clearColor);
	
	struct PassData
	{	
		struct AttachmentReference {
			Id Name;
		};
		GTSL::Array<AttachmentReference, 8> ReadAttachments, WriteAttachments;

		PassType PassType;

		Id ResultAttachment;
	};
	void AddPass(Id name, Id parent, RenderSystem* renderSystem, MaterialSystem* materialSystem, PassData passData);

	void OnResize(RenderSystem* renderSystem, MaterialSystem* materialSystem, const GTSL::Extent2D newSize);

	/**
	 * \brief Enables or disables the rendering of a render pass
	 * \param renderPassName Name of the render Pass to toggle
	 * \param enable Whether to enable(true) or disable(false) the render pass
	 */
	void ToggleRenderPass(Id renderPassName, bool enable);

	MAKE_HANDLE(uint8, IndexStream)
	
	IndexStreamHandle AddIndexStream() {
		auto index = renderState.IndexStreams.GetLength();
		renderState.IndexStreams.EmplaceBack(0);
		return IndexStreamHandle(index);
	}

	void UpdateIndexStream(IndexStreamHandle indexStreamHandle, CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 value);
	void PopIndexStream(IndexStreamHandle indexStreamHandle) { renderState.IndexStreams[indexStreamHandle()] = 0; renderState.IndexStreams.PopBack(); }

	void BindData(const RenderSystem* renderSystem, const MaterialSystem* materialSystem, CommandBuffer commandBuffer, Buffer buffer);

	void PopData() { renderState.Offset -= 4; }

	void OnRenderEnable(TaskInfo taskInfo, bool oldFocus);
	void OnRenderDisable(TaskInfo taskInfo, bool oldFocus);

	using LayerHandle = GTSL::Tree<Layer, BE::PAR>::Node*;
	
	[[nodiscard]] LayerHandle AddLayer(const Id name, const Id parent, const LayerType layerType) {
		GTSL::Tree<Layer, BE::PAR>::Node* layerHandle;

		InternalLayerType internalLayer = InternalLayerType::LAYER;

		switch (layerType) {
		case LayerType::DISPATCH:
			internalLayer = InternalLayerType::DISPATCH;
			break;
		case LayerType::RAY_TRACE:
			internalLayer = InternalLayerType::DISPATCH;
			break;
		case LayerType::MATERIAL:
		case LayerType::MESHES:
			internalLayer = InternalLayerType::MATERIAL_AND_MESHES;
			break;
		case LayerType::RENDER_PASS:
			internalLayer = InternalLayerType::RENDER_PASS;
			break;
		case LayerType::LAYER:
			internalLayer = InternalLayerType::LAYER;
			break;
		}
		
		if (parent()) {
			layerHandle = renderingTree.AddChild(nullptr);
		} else {
			layerHandle = renderingTree.AddChild(renderingTreeMap[name]);
		}

		layerHandle->Data.Type = layerType;
		layerHandle->Data.PrivateType = internalLayer;
		layerHandle->Data.Name = name;
		
		renderingTreeMap.Emplace(name, LayerHandle(layerHandle));

		return LayerHandle(layerHandle);
	}

	auto GetLayer(LayerHandle layerHandle) { return &layerHandle->Data; }
	auto GetLayer(const Id layerHandle) { return GetLayer(renderingTreeMap[layerHandle]); }
	
	void AddMesh(LayerHandle layerHandle, RenderSystem::MeshHandle meshHandle, MaterialInstanceHandle materialInstanceHandle, uint32 instanceCount, GTSL::Range<const GAL::ShaderDataType*> meshVertexLayout) {
		auto layer = GetLayer(layerHandle);
		//layer->Material.
		
		//auto& material = materials[materialHandle.MaterialIndex];
		//auto& materialInstance = material.MaterialInstances[materialHandle.MaterialInstanceIndex];
		//
		//bool found = false;
		//uint16 vertexGroupIndex = 0;
		//
		//for (const auto& e : material.VertexDescriptors) {
		//	if (CompareContents(GTSL::Range<const GAL::ShaderDataType*>(e.begin(), e.end()), vertexDescriptor)) { found = true; break; }
		//
		//	++vertexGroupIndex;
		//}
		//
		//if (found)
		//	materialInstance.VertexGroups[vertexGroupIndex].Meshes.EmplaceBack(meshHandle, 1, instanceIndex);
	}

	auto GetSceneRenderPass() const { return sceneRenderPass; }
	auto GetStaticMeshRenderGroup() const { return staticMeshRenderGroup; }

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
	
	struct RenderState
	{
		Id BoundRenderGroup;
		GTSL::Array<uint32, 8> IndexStreams; // MUST be 4 bytes or push constant reads will be messed up
		//PipelineLayout PipelineLayout;
		GAL::ShaderStage ShaderStages;
		Id ActiveRenderPass; PassType ActiveRenderPassType;
		uint8 Offset = 0;
		Id PipelineLayout;
	} renderState;
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 32>, BE::PersistentAllocatorReference> setupSystemsAccesses;
	
	GTSL::FlatHashMap<Id, SystemHandle, BE::PersistentAllocatorReference> renderManagers;
	
	Id resultAttachment;
	
	LayerHandle staticMeshRenderGroup, sceneRenderPass;
	LayerHandle globalData;
	LayerHandle cameraDataLayer;
	
	using RenderPassFunctionType = GTSL::FunctionPointer<void(GameInstance*, RenderSystem*, MaterialSystem*, CommandBuffer, Id)>;
	
	//GTSL::StaticMap<Id, RenderPassFunctionType, 8> renderPassesFunctions;

	void renderUI(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);

	void transitionImages(CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, const LayerHandle renderPassId);

	struct ShaderLoadInfo
	{
		ShaderLoadInfo() = default;
		ShaderLoadInfo(ShaderLoadInfo&& other) noexcept : Buffer(GTSL::MoveRef(other.Buffer)), Component(other.Component) {}
		GTSL::Buffer<BE::PAR> Buffer; uint32 Component;
	};

	void onShaderInfosLoaded(TaskInfo taskInfo, MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaderInfos, ShaderLoadInfo shaderLoadInfo);
	void onShadersLoaded(TaskInfo taskInfo, MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaders, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo);

	GTSL::Tree<Layer, BE::PAR> renderingTree;
	GTSL::FlatHashMap<Id, LayerHandle, BE::PAR> renderingTreeMap;
	
	//MATERIAL STUFF
	struct RayTracingPipelineData {
		struct ShaderGroupData {
			uint32 RoundedEntrySize = 0;
			BufferHandle Buffer;

			MemberHandle<void*> EntryHandle;
			MemberHandle<GAL::ShaderHandle> ShaderHandle;
			MemberHandle<RenderSystem::BufferAddress> BufferBufferReferencesMemberHandle;
			//uint32 Instances = 0;

			struct ShaderRegisterData {
				struct BufferPatchData {
					Id Buffer;
					bool Has = false;
				};
				GTSL::Array<BufferPatchData, 8> Buffers;
			};
			
			GTSL::Vector<ShaderRegisterData, BE::PAR> Shaders;
		} ShaderGroups[4];

		Pipeline Pipeline;
	};
	GTSL::KeepVector<RayTracingPipelineData, BE::PAR> rayTracingPipelines;

	uint32 textureIndex = 0, imageIndex = 0;
	
	struct CreateTextureInfo
	{
		Id TextureName;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager = nullptr;
		MaterialInstanceHandle MaterialHandle;
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

	struct MaterialInstance
	{
		Id Name;
		uint8 Counter = 0, Target = 0;
	};
	//GTSL::KeepVector<MaterialInstance, BE::PAR> materialInstances;
	
	struct MaterialData {
		Id Name;		
		Id RenderGroup;
		GTSL::Vector<MaterialInstance, BE::PAR> MaterialInstances;
		GTSL::StaticMap<Id, MemberHandle<uint32>, 16> ParametersHandles;
		struct Permutation {
			Pipeline Pipeline;
		};
		GTSL::Array<Permutation, 8> VertexGroups;
		GTSL::Array<GTSL::Array<GAL::ShaderDataType, 20>, 8> VertexDescriptors;		
		GTSL::Array<MaterialResourceManager::Parameter, 16> Parameters;
		MemberHandle<void*> MaterialInstancesMemberHandle;
		BufferHandle BufferHandle;
	};
	GTSL::KeepVector<MaterialData, BE::PAR> materials;
	GTSL::FlatHashMap<Id, uint32, BE::PAR> materialsByName;
	
	//GTSL::KeepVector<MaterialInstanceData, BE::PAR> materialInstances;
	//GTSL::FlatHashMap<Id, uint32, BE::PAR> loadedMaterials;
	//GTSL::FlatHashMap<Id, uint32, BE::PAR> loadedMaterialInstances;
	//GTSL::FlatHashMap<Id, MaterialInstanceData, BE::PAR> awaitingMaterialInstances;
	//GTSL::FlatHashMap<Id, uint32, BE::PAR> materialInstancesByName;

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
	GTSL::KeepVector<GTSL::Vector<MaterialInstanceHandle, BE::PAR>, BE::PersistentAllocatorReference> pendingMaterialsPerTexture;
	
	void addPendingMaterialToTexture(uint32 texture, MaterialInstanceHandle material) {
		pendingMaterialsPerTexture[texture].EmplaceBack(material);
	}

	struct APIRenderPassData {
		Id Name;
		RenderPass RenderPass;
		FrameBuffer FrameBuffer;
		GTSL::Array<Id, 16> UsedAttachments;
	};
	GTSL::Array<APIRenderPassData, 16> apiRenderPasses;

	GTSL::Array<LayerHandle, 16> renderPasses; //TODO: GUARENTEE ORDERING
	struct SubPass {
		Id Name;
		uint8 DepthAttachment;
	};
	GTSL::Array<GTSL::Array<SubPass, 16>, 16> subPasses;
	
	struct Attachment
	{
		RenderSystem::TextureHandle TextureHandle;

		Id Name;
		GAL::TextureUse Uses;
		GAL::TextureLayout Layout;
		GAL::PipelineStage ConsumingStages;
		GAL::AccessType WriteAccess;
		GTSL::RGBA ClearColor;
		GAL::FormatDescriptor FormatDescriptor;
		uint32 ImageIndex;
	};
	GTSL::StaticMap<Id, Attachment, 32> attachments;

	void updateImage(Attachment& attachment, GAL::TextureLayout textureLayout, GAL::PipelineStage stages, GAL::AccessType writeAccess) {
		attachment.Layout = textureLayout; attachment.ConsumingStages = stages; attachment.WriteAccess = writeAccess;
	}

	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureInfoLoadHandle;
	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureLoadHandle;
	DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, ShaderLoadInfo> onShaderInfosLoadHandle;
	DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, GTSL::Range<byte*>, ShaderLoadInfo> onShadersLoadHandle;

	[[nodiscard]] const RenderPass* getAPIRenderPass(const Id renderPassName) const {
		return &apiRenderPasses[renderingTreeMap[renderPassName]->Data.RenderPass.APIRenderPass].RenderPass;
	}
	[[nodiscard]] uint8 getAPISubPassIndex(const Id renderPass) const {
		uint8 i = 0;
		for (auto& e : subPasses[renderingTreeMap[renderPass]->Data.RenderPass.APIRenderPass]) { if (e.Name == renderPass) { return i; } }
	}
	[[nodiscard]] FrameBuffer getFrameBuffer(const uint8 rp) const { return apiRenderPasses[rp].FrameBuffer; }
};
