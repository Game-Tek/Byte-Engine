#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Array.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/FunctionPointer.hpp>
#include <GTSL/StaticMap.hpp>
#include <GTSL/Tree.hpp>

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

class RenderOrchestrator : public System
{
public:
	enum class PassType : uint8 {
		RASTER, COMPUTE, RAY_TRACING
	};
	
	enum class LayerType {
		DISPATCH, RAY_TRACE, MATERIAL, MESHES, RENDER_PASS, LAYER
	};

protected:
	enum class InternalLayerType {
		DISPATCH, RAY_TRACE, MATERIAL, MESH, RENDER_PASS, LAYER, VERTEX_LAYOUT,
		MATERIAL_INSTANCE
	};

	struct AttachmentData {
		Id Name;
		GAL::TextureLayout Layout;
		GAL::PipelineStage ConsumingStages;
		GAL::AccessType Access;
	};

	struct APIRenderPassData {
		uint8 APISubPass = 0, SubPassCount = 0;
		RenderPass RenderPass;
		FrameBuffer FrameBuffer[MAX_CONCURRENT_FRAMES];
	};
	
	struct InternalLayer {
		InternalLayerType Type; GTSL::ShortString<32> Name; uint64 Key; uint32 Index;

		struct MeshData {
			RenderSystem::MeshHandle Handle;
			uint32 InstanceCount = 0;
		};
		
		struct MaterialData {
			MaterialInstanceHandle MaterialHandle;
		};

		struct VertexLayoutData {
			uint8 VertexLayoutIndex;
		};

		struct DispatchData {
			GTSL::Extent3D DispatchSize;
		};

		struct RayTraceData {
			GTSL::Extent3D DispatchSize;
		};
		
		struct RenderPassData {
			bool Enabled = false;

			PassType Type;
			GTSL::Array<AttachmentData, 8> Attachments;

			GAL::PipelineStage PipelineStages;
			SetHandle AttachmentsSetHandle;
			MemberHandle<uint32> AttachmentsIndicesHandle;
			BufferHandle BufferHandle;

			union {
				APIRenderPassData APIRenderPass;
			};
		};

		struct LayerData {
			BufferHandle BufferHandle;
			bool Indexed;
		};
		
		union {
			MaterialData Material;
			MeshData Mesh;
			VertexLayoutData VertexLayout;
			RenderPassData RenderPass;
			DispatchData Dispatch;
			RayTraceData RayTrace;
			LayerData Layer;
		};

		InternalLayer() {}
		
		~InternalLayer() {			
			switch (Type) {
			case InternalLayerType::DISPATCH: GTSL::Destroy(Dispatch); break;
			case InternalLayerType::RAY_TRACE: GTSL::Destroy(RayTrace); break;
			case InternalLayerType::MATERIAL: GTSL::Destroy(Material); break;
			case InternalLayerType::MESH: GTSL::Destroy(Mesh); break;
			case InternalLayerType::RENDER_PASS: GTSL::Destroy(RenderPass); break;
			case InternalLayerType::LAYER: GTSL::Destroy(Layer); break;
			case InternalLayerType::VERTEX_LAYOUT: GTSL::Destroy(VertexLayout); break;
			default: ;
			}
		}
	};

	using InternalLayerHandle = GTSL::Tree<InternalLayer, BE::PAR>::Node*;
	
	struct PublicLayer {
		LayerType Type;
		Id Name; uint32 Index;
		uint64 Key;

		GTSL::StaticMap<uint64, void*, 8> PublicChildrenMap;

		struct InternalNodeData {
			InternalLayerHandle InternalNode;
			GTSL::StaticMap<uint64, InternalLayerHandle, 8> ChildrenMap;
		};
		GTSL::Array<InternalNodeData, 8> InternalSiblings;
		
		PublicLayer() {
		}
		
		struct MeshInstanceData {
			RenderSystem::MeshHandle Handle;
			uint32 InstanceCount;
		};

		struct MeshData {
			RenderSystem::MeshHandle Mesh;
			uint8 VertexGroup;
		};
		
		struct MaterialData {
			MaterialInstanceHandle MaterialHandle;
		};

		struct DispatchData {
			GTSL::Extent3D DispatchSize;
		};

		struct RayTraceData {
			GTSL::Extent3D DispatchSize;
		};

		struct RenderPassData {			
			bool Enabled = false;

			PassType PassType;
			GTSL::Array<AttachmentData, 8> Attachments;

			GAL::PipelineStage PipelineStages;
			SetHandle AttachmentsSetHandle;
			MemberHandle<uint32> AttachmentsIndicesHandle;
			BufferHandle BufferHandle;

			union {
				APIRenderPassData APIRenderPass;
			};
		};

		struct LayerData {
			BufferHandle BufferHandle;
			bool Indexed;
		};
		
		union {
			//MaterialData Material;
			//DispatchData Dispatch;
			//RayTraceData RayTrace;
			//RenderPassData RenderPass;
			//MeshData Mesh;
			//LayerData Layer;
		};

		~PublicLayer() {
			//switch (Type) {
			//case LayerType::DISPATCH: Dispatch.~DispatchData();  break;
			//case LayerType::RAY_TRACE: RayTrace.~RayTraceData(); break;
			//case LayerType::MATERIAL: Material.~MaterialData(); break;
			//case LayerType::MESHES: Mesh.~MeshData(); break;
			//case LayerType::RENDER_PASS: RenderPass.~RenderPassData(); break;
			//case LayerType::LAYER: Layer.~LayerData(); break;
			//default:;
			//}
		}
	};

public:
	using LayerHandle = GTSL::Tree<PublicLayer, BE::PAR>::Node*;

private:	
	GTSL::Tree<InternalLayer, BE::PAR>::Node* addInternalLayer(const uint64 key, LayerHandle publicSibling, LayerHandle publicParent, InternalLayerType type, uint8 index) {
		InternalLayerHandle layerHandle = nullptr;

		
		if (publicParent) {
			if (index == 0xFF) { index = publicParent->Data.InternalSiblings.GetLength() - 1; }
			
			if (publicParent->Data.InternalSiblings[index].ChildrenMap.Find(key)) { //do parent thing, where sibling becomes parent
				layerHandle = publicParent->Data.InternalSiblings[index].ChildrenMap.At(key);
				return layerHandle;
			}
			else {
				layerHandle = internalRenderingTree.AddChild(publicParent->Data.InternalSiblings[index].InternalNode);
				publicParent->Data.InternalSiblings[index].ChildrenMap.Emplace(key, layerHandle);
				
				if (publicSibling) {
					auto& sibling = publicSibling->Data.InternalSiblings.EmplaceBack();
					sibling.InternalNode = layerHandle;
				}
				else {
					auto& sibling = publicParent->Data.InternalSiblings.EmplaceBack();
					sibling.InternalNode = layerHandle;
				}
				
			}
		}
		else {
			layerHandle = internalRenderingTree.AddChild(nullptr);
			auto& sibling = publicSibling->Data.InternalSiblings.EmplaceBack();
			sibling.InternalNode = layerHandle;
		}

		layerHandle->Data.Type = type;
		layerHandle->Data.Key = key;

		return layerHandle;
	}

public:
	RenderOrchestrator() : System("RenderOrchestrator") {}
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo);

	void AddRenderManager(GameInstance* gameInstance, const Id renderManager, const SystemHandle systemReference);
	void RemoveRenderManager(GameInstance* gameInstance, const Id renderGroupName, const SystemHandle systemReference);
	LayerHandle GetCameraDataLayer() const { return cameraDataLayer; }

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
		auto& renderPass = renderPasses.At(renderPassName)->Data.InternalSiblings.back().InternalNode->Data.RenderPass;
		uint8 count = 0;
		for(const auto& e : renderPass.Attachments) {
			if(e.Access & GAL::AccessTypes::WRITE)
				if (e.Layout == GAL::TextureLayout::ATTACHMENT || e.Layout == GAL::TextureLayout::GENERAL) { ++count; }
		}
		return count;
	}

	void AddAttachment(Id name, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type, GTSL::RGBA clearColor);
	
	struct PassData {
		struct AttachmentReference {
			Id Name;
		};
		GTSL::Array<AttachmentReference, 8> ReadAttachments, WriteAttachments;

		PassType PassType;
	};
	void AddPass(Id name, LayerHandle parent, RenderSystem* renderSystem, MaterialSystem* materialSystem, PassData passData);

	void OnResize(RenderSystem* renderSystem, MaterialSystem* materialSystem, const GTSL::Extent2D newSize);

	/**
	 * \brief Enables or disables the rendering of a render pass
	 * \param renderPassName Name of the render Pass to toggle
	 * \param enable Whether to enable(true) or disable(false) the render pass
	 */
	void ToggleRenderPass(LayerHandle renderPassName, bool enable);

	MAKE_HANDLE(uint8, IndexStream) MAKE_HANDLE(uint8, DataStream)

	void OnRenderEnable(TaskInfo taskInfo, bool oldFocus);
	void OnRenderDisable(TaskInfo taskInfo, bool oldFocus);

	[[nodiscard]] LayerHandle AddLayer(const uint64 name, const LayerHandle parent, const LayerType layerType) {
		GTSL::Tree<PublicLayer, BE::PAR>::Node* layerHandle;

		if (parent) {
			if (const auto layerHandleSearch = parent->Data.PublicChildrenMap.Find(name)) {
				layerHandle = static_cast<LayerHandle>(parent->Data.PublicChildrenMap.At(name));
				return layerHandle;
			} else {				
				layerHandle = renderingTree.AddChild(parent);
				parent->Data.PublicChildrenMap.Emplace(name, layerHandle);
			}
		} else {
			layerHandle = renderingTree.AddChild(nullptr);
		}

		auto& data = layerHandle->Data;		

		layerHandle->Data.Type = layerType;
		layerHandle->Data.Key = name;
		
		switch (data.Type) {
		case LayerType::DISPATCH: {
			addInternalLayer(data.Key, layerHandle, parent, InternalLayerType::DISPATCH, 0xFF);
			break;
		}
		case LayerType::RAY_TRACE: {
			addInternalLayer(data.Key, layerHandle, parent, InternalLayerType::RAY_TRACE, 0xFF);
			break;
		}
		case LayerType::MATERIAL: {
			break;
		}
		case LayerType::MESHES: {
			break;
		}
		case LayerType::RENDER_PASS: {
			addInternalLayer(data.Key, layerHandle, parent, InternalLayerType::RENDER_PASS, 0xFF);
			break;
		}
		case LayerType::LAYER: {
			addInternalLayer(data.Key, layerHandle, parent, InternalLayerType::LAYER, 0xFF);
			break;
		}
		}
		
		return LayerHandle(layerHandle);
	}

	[[nodiscard]] LayerHandle AddLayer(const Id name, const LayerHandle parent, const LayerType layerType) {
		auto l = AddLayer(name(), parent, layerType);
		l->Data.Name = name;
		l->Data.InternalSiblings.back().InternalNode->Data.Name = name.GetString();
		return l;
	}

	[[nodiscard]] LayerHandle AddLayer(const Id name, const BufferHandle bufferHandle, bool indexed, const LayerHandle parent) {
		auto l = AddLayer(name(), parent, LayerType::LAYER);
		l->Data.Name = name;
		l->Data.InternalSiblings.back().InternalNode->Data.Name = name.GetString();
		l->Data.InternalSiblings.back().InternalNode->Data.Layer.BufferHandle = bufferHandle;
		l->Data.InternalSiblings.back().InternalNode->Data.Layer.Indexed = indexed;
		return l;
	}
	
	auto GetLayer(LayerHandle layerHandle) { return &layerHandle->Data; }
	
	LayerHandle AddMaterial(LayerHandle layerHandle, MaterialInstanceHandle materialHandle) {
		auto materialKey = (uint64)materialHandle.MaterialInstanceIndex << 32 | materialHandle.MaterialIndex;
		
		auto layer = AddLayer(materialKey, layerHandle, LayerType::MATERIAL);

		auto material = addInternalLayer(materialKey, layer, layerHandle, InternalLayerType::MATERIAL, 0);
		auto materialInstance = addInternalLayer(materialHandle.MaterialInstanceIndex, nullptr, layer, InternalLayerType::MATERIAL_INSTANCE, 0);
		
		material->Data.Name = materials[materialHandle.MaterialIndex].Name.GetString();
		material->Data.Material.MaterialHandle = materialHandle;

		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Material Instance #"); name += materialHandle.MaterialInstanceIndex;
			materialInstance->Data.Name = name;
		}
		
		materialInstance->Data.Index = materialHandle.MaterialInstanceIndex;
		
		return layer;
	}
	
	void AddMesh(LayerHandle layerHandle, RenderSystem::MeshHandle meshHandle, uint32 index, GTSL::Range<const GAL::ShaderDataType*> meshVertexLayout) {
		auto layer = AddLayer(meshHandle(), layerHandle, LayerType::MESHES);

		bool foundLayout = false; uint8 layoutIndex = 0;

		for (; layoutIndex < vertexLayouts.GetLength(); ++layoutIndex) {
			if (vertexLayouts[layoutIndex].GetLength() != meshVertexLayout.ElementCount()) { continue; }

			foundLayout = true;

			for (uint8 i = 0; i < meshVertexLayout.ElementCount(); ++i) {
				if (meshVertexLayout[i] != vertexLayouts[layoutIndex][i]) { foundLayout = false; break; }
			}

			if (foundLayout) { break; }

			++layoutIndex;
		}

		if(!foundLayout) {
			foundLayout = true;
			layoutIndex = vertexLayouts.GetLength();
			auto& vertexLayout = vertexLayouts.EmplaceBack();
			
			for(auto e : meshVertexLayout) {
				vertexLayout.EmplaceBack(e);
			}
		}

		auto vertexLayoutNode = addInternalLayer(layoutIndex, nullptr, layerHandle, InternalLayerType::VERTEX_LAYOUT, 0);

		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Vertex Layout #"); name += static_cast<uint32>(layoutIndex);
			vertexLayoutNode->Data.Name = name;
		}
		
		auto meshLayer = addInternalLayer(meshHandle(), layer, layerHandle, InternalLayerType::MESH, 1);
		
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Mesh #"); name += static_cast<uint32>(meshHandle());
			meshLayer->Data.Name = name;
		}
		
		vertexLayoutNode->Data.VertexLayout.VertexLayoutIndex = layoutIndex;
		meshLayer->Data.Index = index;
		meshLayer->Data.Mesh.Handle = meshHandle;
		++meshLayer->Data.Mesh.InstanceCount;
	}

	auto GetSceneRenderPass() const { return sceneRenderPass; }

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

	GTSL::Array<GTSL::Array<GAL::ShaderDataType, 24>, 32> vertexLayouts;
	
	struct RenderState {
		uint8 APISubPass = 0, MaxAPIPass = 0;
		GAL::ShaderStage ShaderStages;
		uint8 streamsCount = 0, buffersCount = 0, indecesCount = 0;
		Id PipelineLayout = "GlobalData";

		bool slotsWrittenTo[32]{ false };
		
		GTSL::Array<IndexStreamHandle, 16> indexStreams;
		const GTSL::Tree<InternalLayer, BE::PAR>::Node* LastMaterial;

		IndexStreamHandle AddIndexStream() {			
			++indecesCount;
			return indexStreams.EmplaceBack(IndexStreamHandle(streamsCount++));
		}

		void UpdateIndexStream(IndexStreamHandle indexStreamHandle, CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 value);
		
		void PopIndexStream(IndexStreamHandle indexStreamHandle) {
			--streamsCount; --indecesCount;
			BE_ASSERT(indexStreamHandle() == streamsCount);
		}

		DataStreamHandle AddDataStream() {
			++buffersCount;
			return DataStreamHandle(streamsCount++);
		}
		
		void BindData(DataStreamHandle dataStreamHandle, const RenderSystem* renderSystem, const MaterialSystem* materialSystem, CommandBuffer commandBuffer, GPUBuffer buffer);
		
		void PopData(DataStreamHandle dataStreamHandle) {
			--streamsCount; --buffersCount;
			BE_ASSERT(dataStreamHandle() == streamsCount);
		}
	};
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 32>, BE::PersistentAllocatorReference> setupSystemsAccesses;
	
	GTSL::FlatHashMap<Id, SystemHandle, BE::PersistentAllocatorReference> renderManagers;
	
	Id resultAttachment;
	
	LayerHandle sceneRenderPass, globalData, cameraDataLayer;
	
	using RenderPassFunctionType = GTSL::FunctionPointer<void(GameInstance*, RenderSystem*, MaterialSystem*, CommandBuffer, Id)>;
	
	//GTSL::StaticMap<Id, RenderPassFunctionType, 8> renderPassesFunctions;

	void renderUI(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);

	void transitionImages(CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, const InternalLayerHandle renderPassId);

	struct ShaderLoadInfo
	{
		ShaderLoadInfo() = default;
		ShaderLoadInfo(ShaderLoadInfo&& other) noexcept : Buffer(GTSL::MoveRef(other.Buffer)), Component(other.Component) {}
		GTSL::Buffer<BE::PAR> Buffer; uint32 Component;
	};

	void onShaderInfosLoaded(TaskInfo taskInfo, MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaderInfos, ShaderLoadInfo shaderLoadInfo);
	void onShadersLoaded(TaskInfo taskInfo, MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaders, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo);

	GTSL::Tree<PublicLayer, BE::PAR> renderingTree;
	GTSL::Tree<InternalLayer, BE::PAR> internalRenderingTree;

	GTSL::StaticMap<Id, LayerHandle, 16> renderPasses;

	GTSL::Extent2D sizeHistory[MAX_CONCURRENT_FRAMES];
	
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
		GTSL::Vector<MaterialInstance, BE::PAR> MaterialInstances;
		GTSL::StaticMap<Id, MemberHandle<uint32>, 16> ParametersHandles;
		struct Permutation {
			Pipeline Pipeline;
		};
		GTSL::StaticMap<uint8, Permutation, 8> VertexGroups;
		GTSL::Array<MaterialResourceManager::Parameter, 16> Parameters;
		MemberHandle<void*> MaterialInstancesMemberHandle;
		BufferHandle BufferHandle;
	};
	GTSL::KeepVector<MaterialData, BE::PAR> materials;
	GTSL::FlatHashMap<Id, uint32, BE::PAR> materialsByName;

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
	
	struct Attachment {
		RenderSystem::TextureHandle TextureHandle[MAX_CONCURRENT_FRAMES];

		Id Name;
		GAL::TextureUse Uses; GAL::TextureLayout Layout;
		GAL::PipelineStage ConsumingStages; GAL::AccessType AccessType;
		GTSL::RGBA ClearColor; GAL::FormatDescriptor FormatDescriptor;
		uint32 ImageIndex;
	};
	GTSL::StaticMap<Id, Attachment, 32> attachments;

	void updateImage(Attachment& attachment, GAL::TextureLayout textureLayout, GAL::PipelineStage stages, GAL::AccessType writeAccess) {
		attachment.Layout = textureLayout; attachment.ConsumingStages = stages; attachment.AccessType = writeAccess;
	}

	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureInfoLoadHandle;
	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureLoadHandle;
	DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, ShaderLoadInfo> onShaderInfosLoadHandle;
	DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, GTSL::Range<byte*>, ShaderLoadInfo> onShadersLoadHandle;

	[[nodiscard]] const RenderPass* getAPIRenderPass(const Id renderPassName) const {
		return &renderPasses.At(renderPassName)->Data.InternalSiblings.back().InternalNode->Data.RenderPass.APIRenderPass.RenderPass;
	}
	
	[[nodiscard]] uint8 getAPISubPassIndex(const Id renderPass) const {
		return renderPasses.At(renderPass)->Data.InternalSiblings.back().InternalNode->Data.RenderPass.APIRenderPass.APISubPass;
	}
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
	RenderOrchestrator::LayerHandle staticMeshRenderGroup;
	BufferHandle bufferHandle;
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
