#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Array.hpp>
#include <GTSL/HashMap.h>
#include <GTSL/FunctionPointer.hpp>
#include <GTSL/StaticMap.hpp>
#include <GTSL/PagedVector.h>
#include <GTSL/SparseVector.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/Bitfield.h>

#include "ByteEngine/Id.h"
#include "RenderSystem.h"
#include "RenderTypes.h"
#include "StaticMeshRenderGroup.h"
#include "ByteEngine/Game/Tasks.h"
#include "ByteEngine/Resources/ShaderResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

class RenderOrchestrator;
class RenderState;
class RenderSystem;
class RenderGroup;
struct TaskInfo;

class RenderManager : public System
{
public:
	RenderManager(const InitializeInfo& initializeInfo, const char8_t* name) : System(initializeInfo, name) {}
	
	virtual void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) = 0;

	struct SetupInfo
	{
		GameInstance* GameInstance;
		RenderSystem* RenderSystem;
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
	
	enum class LayerType : uint8 {
		DISPATCH, RAY_TRACE, MATERIAL, MESHES, RENDER_PASS, LAYER
	};

	MAKE_HANDLE(uint32, Layer);

protected:
	enum class InternalLayerType {
		DISPATCH, RAY_TRACE, MATERIAL, MESH, RENDER_PASS, LAYER, MATERIAL_INSTANCE
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
		InternalLayerType Type;
		uint16 DirectChildren = 0;
		uint32 Offset = ~0U;
		GTSL::ShortString<32> Name;
		bool Enabled = true;

		uint32 Next = ~0U;

		struct MeshData {
			RenderSystem::MeshHandle Handle;
			uint32 InstanceCount = 0;
		};
		
		struct MaterialData {
			MaterialInstanceHandle MaterialHandle;
			uint8 VertexLayoutIndex;
		};

		struct DispatchData {
			GTSL::Extent3D DispatchSize;
		};

		struct RayTraceData {
			GTSL::Extent3D DispatchSize;
			uint32 PipelineIndex = 0;
		};
		
		struct RenderPassData {
			PassType Type;
			GTSL::Array<AttachmentData, 8> Attachments;
			GAL::PipelineStage PipelineStages;

			RenderPassData() : Type(PassType::RASTER), Attachments(), PipelineStages(), APIRenderPass() {
			}
			
			union {
				APIRenderPassData APIRenderPass;
			};
		};

		struct LayerData {
			BufferHandle BufferHandle;
		};
		
		union {
			MaterialData Material;
			MeshData Mesh;
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
			default: ;
			}
		}
	};

	MAKE_HANDLE(uint32, InternalLayer)
	
	struct PublicLayer {
		LayerType Type; uint8 Level = 0;
		Id Name;
		uint32 Offset = ~0U;

		LayerHandle Parent;
		uint32 Children = 0;
		uint32 InstanceCount = 0;

		InternalLayerHandle EndOfChain;

		GTSL::StaticMap<uint64, LayerHandle, 8> ChildrenMap;

		struct InternalNodeData {
			InternalLayerHandle InternalNode;
			GTSL::StaticMap<uint64, InternalLayerHandle, 8> ChildrenMap;
		};
		GTSL::Array<InternalNodeData, 8> InternalSiblings;
	};	

	PublicLayer& getLayer(const LayerHandle layerHandle) {
		return renderingTree[layerHandle()];
	}
	
	InternalLayer& getLayer(const InternalLayerHandle internal_layer_handle) {
		return internalRenderingTree[internal_layer_handle()];
	}

	const InternalLayer& getLayer(const InternalLayerHandle internal_layer_handle) const {
		return internalRenderingTree[internal_layer_handle()];
	}

public:
	struct Member {
		enum class DataType : uint8 {
			FLOAT32, INT32, UINT32, UINT64, MATRIX4, MATRIX3X4, FVEC4, FVEC2, STRUCT, PAD,
			SHADER_HANDLE
		};

		Member() = default;
		Member(const uint32 count, const DataType type) : Count(count), Type(type) {}

		uint32 Count = 1;
		DataType Type = DataType::PAD;
	};

	template<typename T>
	struct MemberHandle {
		uint64 Hash = 0; uint32 Offset = 0, Size = 0;

		MemberHandle operator[](const uint32 index) {
			return MemberHandle{ Hash, Offset + Size * index, Size };
		}
	};
	
	struct MemberInfo : Member
	{
		MemberInfo() = default;
		MemberInfo(const uint32 count) : Member(count, Member::DataType::PAD) {}
		MemberInfo(MemberHandle<uint32>* memberHandle, const uint32 count) : Member(count, DataType::UINT32), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GAL::DeviceAddress>* memberHandle, const uint32 count) : Member(count, DataType::UINT64), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GTSL::Matrix4>* memberHandle, const uint32 count) : Member(count, DataType::MATRIX4), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GTSL::Matrix3x4>* memberHandle, const uint32 count) : Member(count, DataType::MATRIX3X4), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GAL::ShaderHandle>* memberHandle, const uint32 count) : Member(count, DataType::SHADER_HANDLE), Handle(memberHandle) {}
		MemberInfo(MemberHandle<void*>* memberHandle, const uint32 count, GTSL::Range<MemberInfo*> memberInfos) : Member(count, DataType::STRUCT), Handle(memberHandle), MemberInfos(memberInfos) {}

		void* Handle = nullptr;
		GTSL::Range<MemberInfo*> MemberInfos;
	};

private:	
	InternalLayer& addInternalLayer(const uint64 key, LayerHandle publicSiblingHandle, LayerHandle publicParentHandle, InternalLayerType type) {
		InternalLayerHandle layerHandle;
		InternalLayer* layer = nullptr;
		
		if (publicParentHandle) {
			auto& publicParent = getLayer(publicParentHandle);
			
			auto internalParentHandle = publicParent.InternalSiblings.back().InternalNode;
			auto& internalParent = getLayer(internalParentHandle);
			internalParent.DirectChildren++;
			
			if (publicParent.InternalSiblings.back().ChildrenMap.Find(key)) {
				layerHandle = publicParent.InternalSiblings.back().ChildrenMap.At(key);
				return getLayer(layerHandle);
			}			
			
			uint32 insertPosition = internalRenderingTree.Emplace();
			layerHandle = InternalLayerHandle(insertPosition);
			layer = &internalRenderingTree[insertPosition];

			LayerHandle p = publicParentHandle;

			while (getLayer(p).Parent) {
				p = getLayer(p).Parent;
			}
			
			if (internalParent.Next == ~0U) {
				internalParent.Next = layerHandle();
				getLayer(p).EndOfChain = layerHandle;
			} else {
				getLayer(getLayer(p).EndOfChain).Next = layerHandle();
			}
			
			publicParent.InternalSiblings.back().ChildrenMap.Emplace(key, layerHandle);
			
			//if (publicSiblingHandle) {
				auto& sibling = getLayer(publicSiblingHandle).InternalSiblings.EmplaceBack();
				sibling.InternalNode = layerHandle;
			//} else {
			//	auto& sibling = publicParent.InternalSiblings.EmplaceBack();
			//	sibling.InternalNode = layerHandle;
			//}
			
		} else {
			uint32 insertPosition = internalRenderingTree.Emplace();
			layerHandle = InternalLayerHandle(insertPosition);
			layer = &internalRenderingTree[insertPosition];
			auto& sibling = getLayer(publicSiblingHandle).InternalSiblings.EmplaceBack();
			sibling.InternalNode = layerHandle;
		}

		layer->Type = type;

		return *layer;
	}

public:
	explicit RenderOrchestrator(const InitializeInfo& initializeInfo);

	MAKE_HANDLE(uint32, Set)

	struct SubSetDescription {
		SetHandle SetHandle; uint32 Subset;
		GAL::BindingType Type;
	};

	MAKE_HANDLE(SubSetDescription, SubSet)

	//template<typename T>
	//struct MemberHandle2
	//{
	//	uint32 BufferIndex = 0, MemberIndirectionIndex = 0;
	//};

	MAKE_HANDLE(uint32, Buffer)
	MAKE_HANDLE(uint64, SetLayout)
	
	void AddData(LayerHandle layer_handle, MemberHandle<void*> memberHandle) {
		auto& publicNode = getLayer(layer_handle);
		auto& privateNode = getLayer(publicNode.InternalSiblings.back().InternalNode);
		publicNode.Offset = renderDataOffset;
		privateNode.Offset = renderDataOffset;
		renderDataOffset += memberHandle.Size;
	}
	
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo);

	void AddRenderManager(GameInstance* gameInstance, const Id renderManager, const SystemHandle systemReference);
	void RemoveRenderManager(GameInstance* gameInstance, const Id renderGroupName, const SystemHandle systemReference);
	LayerHandle GetCameraDataLayer() const { return cameraDataLayer; }

	uint32 renderDataOffset = 0;
	SetLayoutHandle globalSetLayout;
	SetHandle globalBindingsSet;
	RenderAllocation allocation;
	GPUBuffer buffer;

	struct CreateMaterialInfo
	{
		Id MaterialName, InstanceName;
		ShaderResourceManager* ShaderResourceManager = nullptr;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager;
	};
	[[nodiscard]] MaterialInstanceHandle CreateMaterial(const CreateMaterialInfo& info);

	void AddAttachment(Id name, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type, GTSL::RGBA clearColor);
	
	struct PassData {
		struct AttachmentReference {
			Id Name;
		};
		GTSL::Array<AttachmentReference, 8> ReadAttachments, WriteAttachments;

		PassType PassType;
	};
	void AddPass(Id name, LayerHandle parent, RenderSystem* renderSystem, PassData passData);

	void OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize);

	/**
	 * \brief Enables or disables the rendering of a render pass
	 * \param renderPassName Name of the render Pass to toggle
	 * \param enable Whether to enable(true) or disable(false) the render pass
	 */
	void ToggleRenderPass(LayerHandle renderPassName, bool enable);

	MAKE_HANDLE(uint8, IndexStream) MAKE_HANDLE(uint8, DataStream)

	void OnRenderEnable(TaskInfo taskInfo, bool oldFocus);
	void OnRenderDisable(TaskInfo taskInfo, bool oldFocus);

	MemberHandle<void*> MakeMember(const GTSL::Range<MemberInfo*> members) {
		uint64 hash = 0; GAL::BufferUse bufferUses, notBufferFlags;
		
		auto parseMembers = [&](auto&& self, GTSL::Range<MemberInfo*> levelMembers, uint16 level) -> uint32 {
			uint32 size = 0, offset = 0;

			for (uint8 m = 0; m < levelMembers.ElementCount(); ++m) {
				if (levelMembers[m].Type == Member::DataType::PAD) { offset += levelMembers[m].Count; continue; }

				//auto memberDataIndex = bufferData.MemberData.GetLength();
				//auto& member = bufferData.MemberData.EmplaceBack();

				//member.ByteOffsetIntoStruct = offset;
				//member.Level = level;
				//member.Type = levelMembers[m].Type;
				//member.Count = levelMembers[m].Count;

				if (levelMembers[m].Type == Member::DataType::STRUCT) {
					size = self(self, levelMembers[m].MemberInfos, level + 1);
				}
				else {
					if (levelMembers[m].Type == Member::DataType::SHADER_HANDLE) {
						bufferUses |= GAL::BufferUses::SHADER_BINDING_TABLE;
						notBufferFlags |= GAL::BufferUses::ACCELERATION_STRUCTURE; notBufferFlags |= GAL::BufferUses::STORAGE;
					}

					size = dataTypeSize(levelMembers[m].Type);
				}
				
				*static_cast<MemberHandle<byte>*>(levelMembers[m].Handle) = MemberHandle<byte>{ hash, offset, size };

				offset += size * levelMembers[m].Count;
			}

			return offset;
		};

		uint32 bufferSize = parseMembers(parseMembers, members, 0);
		
		//for(auto e : members) {
		//	hash |= static_cast<GTSL::UnderlyingType<decltype(e.Type)>>(e.Type);
		//	hash |= e.Count << 8;
		//}

		return MemberHandle<void*>{ hash, 0, bufferSize };
	}

	void onPush(LayerHandle layer, PublicLayer& publicLayer, uint8 level) {
		publicLayer.Level = level;
		publicLayer.InstanceCount++;
		publicLayer.Parent = LayerHandle();

		//for (uint32 i = layer(); i < internalIndirectionTable.GetLength(); ++i) {
		//	++internalIndirectionTable[i];
		//}
	}

	void onPush(LayerHandle layer, PublicLayer& publicLayer, LayerHandle parent) {
		onPush(layer, publicLayer, getLayer(parent).Level + 1);
		publicLayer.Parent = parent;

		LayerHandle p = parent;

		do {
			++getLayer(p).Children;
			p = getLayer(p).Parent;
		} while (p);
	}
	
	LayerHandle PushNode() {
		auto layerHandle = LayerHandle(renderingTree.GetLength());
		auto& self = renderingTree.EmplaceBack();
		onPush(layerHandle, self, 0);
		return layerHandle;
	}
	
	LayerHandle PushNode(const LayerHandle parent) {
		auto layerHandle = LayerHandle(renderingTree.GetLength());
		auto& self = renderingTree.EmplaceBack();
		onPush(layerHandle, self, parent);
		return layerHandle;
	}
	
	[[nodiscard]] LayerHandle AddLayer(const uint64 key, LayerHandle parent, const LayerType layerType) {
		LayerHandle layerHandle;
		
		if (parent) {			
			if (getLayer(parent).ChildrenMap.Find(key)) {
				auto& pa = getLayer(parent);		
				layerHandle = pa.ChildrenMap.At(key);
				onPush(layerHandle, getLayer(layerHandle), parent);
				return layerHandle;
			}
			
			layerHandle = PushNode(parent);
			getLayer(parent).ChildrenMap.Emplace(key, layerHandle);
			
		} else {
			layerHandle = PushNode();
		}

		auto& data = getLayer(layerHandle);		
		data.Type = layerType;
		
		switch (data.Type) {
		case LayerType::DISPATCH: {
			addInternalLayer(key, layerHandle, parent, InternalLayerType::DISPATCH);
			break;
		}
		case LayerType::RAY_TRACE: {
			addInternalLayer(key, layerHandle, parent, InternalLayerType::RAY_TRACE);
			break;
		}
		case LayerType::MATERIAL: {
			break;
		}
		case LayerType::MESHES: {
			break;
		}
		case LayerType::RENDER_PASS: {
			auto& layer = addInternalLayer(key, layerHandle, parent, InternalLayerType::RENDER_PASS);
			layer.RenderPass = InternalLayer::RenderPassData();
			break;
		}
		case LayerType::LAYER: {
			addInternalLayer(key, layerHandle, parent, InternalLayerType::LAYER);
			break;
		}
		}
		
		return layerHandle;
	}

	[[nodiscard]] LayerHandle AddLayer(const Id name, const LayerHandle parent, const LayerType layerType) {
		auto l = AddLayer(name(), parent, layerType);
		auto& t = getLayer(l);
		t.Name = name; getLayer(t.InternalSiblings.back().InternalNode).Name = name.GetString();
		return l;
	}
	
	LayerHandle AddMaterial(LayerHandle parentHandle, MaterialInstanceHandle materialHandle) {
		auto materialKey = (uint64)materialHandle.MaterialInstanceIndex << 32 | materialHandle.MaterialIndex;

		auto result = nodesByName.TryEmplace(materialKey, 4, GetPersistentAllocator());
		
		if (!result) {
			for(auto& e : result.Get()) 	{
				if(getLayer(e).Parent == parentHandle) {
					return e;
				}
			}
		} else {
			auto layer = AddLayer(materialKey, parentHandle, LayerType::MATERIAL);

			auto& material = addInternalLayer(materialKey, layer, parentHandle, InternalLayerType::MATERIAL);
			auto& materialInstance = addInternalLayer(materialHandle.MaterialInstanceIndex, layer, layer, InternalLayerType::MATERIAL_INSTANCE);
			
			materialInstance.Enabled = true;
			
			material.Name = materials[materialHandle.MaterialIndex].Name.GetString();
			material.Material.MaterialHandle = materialHandle;
			materialInstance.Material.MaterialHandle = materialHandle;

			if constexpr (_DEBUG) {
				GTSL::StaticString<64> name(u8"Material Instance #"); name += materialHandle.MaterialInstanceIndex;
				materialInstance.Name = name;
			}
			return layer;
		}
	}
	
	LayerHandle AddMesh(LayerHandle parentNodeHandle, RenderSystem::MeshHandle meshHandle, GTSL::Range<const GAL::ShaderDataType*> meshVertexLayout, MemberHandle<void*> handle) {
		auto layer = AddLayer(meshHandle(), parentNodeHandle, LayerType::MESHES);

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
		
		auto& meshNode = addInternalLayer(meshHandle(), layer, parentNodeHandle, InternalLayerType::MESH);
		
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name(u8"Mesh #"); name += meshHandle();
			meshNode.Name = name;
		}
	
		meshNode.Mesh.Handle = meshHandle;

		{
			meshNode.Offset = renderDataOffset;
			renderDataOffset += handle.Size;
		}

		return layer;
	}

	template<typename T>
	void Write(const LayerHandle layer, RenderSystem* renderSystem, MemberHandle<T> member, const T& data) {
		BE_ASSERT(getLayer(layer).Offset != ~0U, u8"")
		*reinterpret_cast<T*>(renderSystem->GetBufferPointer(renderBuffers[0].BufferHandle) + getLayer(layer).Offset + member.Offset) = data;
	}
	
	auto GetSceneRenderPass() const { return sceneRenderPass; }

	[[nodiscard]] GPUBuffer GetBuffer(RenderSystem* render_system, BufferHandle bufferHandle) const { return buffers[bufferHandle()].Buffers[render_system->GetCurrentFrame()]; }
	
	void WriteBinding(RenderSystem* render_system, SubSetHandle subSetHandle, uint32 bindingIndex, AccelerationStructure accelerationStructure) {
		for (uint8 f = 0; f < render_system->GetPipelinedFrames(); ++f) {
			descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
		}
	}

	void WriteBinding(SubSetHandle subSetHandle, uint32 bindingIndex, AccelerationStructure accelerationStructure, uint8 f) {
		descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
	}

	GTSL::uint64 GetBufferAddress(RenderSystem* renderSystem, const BufferHandle bufferHandle) const {
		GTSL::uint64 address = 0;
		if (buffers[bufferHandle()].Buffers[renderSystem->GetCurrentFrame()].GetVkBuffer()) {
			address = buffers[bufferHandle()].Buffers[renderSystem->GetCurrentFrame()].GetAddress(renderSystem->GetRenderDevice());
		}
		return address;
	}

	void PushConstant(const RenderSystem* renderSystem, CommandList commandBuffer, SetLayoutHandle layout, uint32 offset, GTSL::Range<const byte*> range) const {
		const auto& set = setLayoutDatas[layout()];
		commandBuffer.UpdatePushConstant(renderSystem->GetRenderDevice(), set.PipelineLayout, offset, range, set.Stage);
	}

	void BindSet(RenderSystem* renderSystem, CommandList commandBuffer, SetHandle setHandle, GAL::ShaderStage shaderStage) {
		if (auto& set = sets[setHandle()]; set.BindingsSet[renderSystem->GetCurrentFrame()].GetHandle()) {
			commandBuffer.BindBindingsSets(renderSystem->GetRenderDevice(), shaderStage, GTSL::Range<BindingsSet*>(1, &set.BindingsSet[renderSystem->GetCurrentFrame()]),
				GTSL::Range<const uint32*>(), set.PipelineLayout, set.Level);
		}
	}

	void WriteBinding(const RenderSystem* renderSystem, SubSetHandle setHandle, RenderSystem::TextureHandle textureHandle, uint32 bindingIndex) {
		GAL::TextureLayout layout; GAL::BindingType bindingType;

		if (setHandle().Type == GAL::BindingType::STORAGE_IMAGE) {
			layout = GAL::TextureLayout::GENERAL;
			bindingType = GAL::BindingType::STORAGE_IMAGE;
		}
		else {
			layout = GAL::TextureLayout::SHADER_READ;
			bindingType = GAL::BindingType::COMBINED_IMAGE_SAMPLER;
		}

		for (uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
			BindingsPool::TextureBindingUpdateInfo info;
			info.TextureView = renderSystem->GetTextureView(textureHandle);
			info.Sampler = renderSystem->GetTextureSampler(textureHandle);
			info.TextureLayout = layout;
			info.FormatDescriptor;

			descriptorsUpdates[f].AddTextureUpdate(setHandle, bindingIndex, info);
		}
	}

	enum class SubSetType : uint8 {
		BUFFER, READ_TEXTURES, WRITE_TEXTURES, RENDER_ATTACHMENT, ACCELERATION_STRUCTURE
	};

	static unsigned long long quickhash64(const GTSL::Range<const byte*> range)
	{ // set 'mix' to some value other than zero if you want a tagged hash          
		const unsigned long long mulp = 2654435789;
		unsigned long long mix = 0;

		mix ^= 104395301;

		for(auto e : range)
			mix += (e * mulp) ^ (mix >> 23);

		return mix ^ (mix << 37);
	}
	
	struct SubSetDescriptor
	{
		SubSetType SubSetType; uint32 BindingsCount;
	};
	SetLayoutHandle AddSetLayout(RenderSystem* renderSystem, SetLayoutHandle parentName, const GTSL::Range<SubSetDescriptor*> subsets) {
		uint64 hash = quickhash64(GTSL::Range<const byte*>(subsets.Bytes(), reinterpret_cast<const byte*>(subsets.begin())));
		
		SetLayoutHandle parentHandle;
		uint32 level;

		if (parentName()) {
			auto& parentSetLayout = setLayoutDatas[parentName()];

			parentHandle = parentName;
			level = parentSetLayout.Level + 1;
		} else {
			parentHandle = SetLayoutHandle();
			level = 0;
		}

		auto& setLayoutData = setLayoutDatas.Emplace(hash);

		setLayoutData.Parent = parentHandle;
		setLayoutData.Level = level;

		GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;

		// Traverse tree to find parent's pipeline layouts
		{
			auto lastSet = parentHandle;

			for (uint8 i = 0; i < level; ++i) { bindingsSetLayouts.EmplaceBack(); }

			for (uint8 i = 0, l = level - 1; i < level; ++i, --l) {
				bindingsSetLayouts[l] = setLayoutDatas[lastSet()].BindingsSetLayout;
				lastSet = setLayoutDatas[lastSet()].Parent;
			}
		}

		setLayoutData.Stage = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::COMPUTE | GAL::ShaderStages::RAY_GEN;

		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> subSetDescriptors;

		for (auto e : subsets) {
			GAL::ShaderStage shaderStage = setLayoutData.Stage;
			GAL::BindingFlag bindingFlags;

			GAL::BindingType bindingType = {};

			if (e.BindingsCount != 1) { bindingFlags = GAL::BindingFlags::PARTIALLY_BOUND; }

			switch (e.SubSetType) {
			case SubSetType::BUFFER: bindingType = GAL::BindingType::STORAGE_BUFFER; break;
			case SubSetType::READ_TEXTURES: bindingType = GAL::BindingType::COMBINED_IMAGE_SAMPLER; break;
			case SubSetType::WRITE_TEXTURES: bindingType = GAL::BindingType::STORAGE_IMAGE; break;
			case SubSetType::RENDER_ATTACHMENT: bindingType = GAL::BindingType::INPUT_ATTACHMENT; break;
			case SubSetType::ACCELERATION_STRUCTURE:
				bindingType = GAL::BindingType::ACCELERATION_STRUCTURE;
				shaderStage = GAL::ShaderStages::RAY_GEN;
				setLayoutData.Stage |= shaderStage;
				break;
			}

			subSetDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ bindingType, shaderStage, e.BindingsCount, bindingFlags });
		}

		setLayoutData.BindingsSetLayout.Initialize(renderSystem->GetRenderDevice(), subSetDescriptors);
		bindingsSetLayouts.EmplaceBack().Initialize(renderSystem->GetRenderDevice(), subSetDescriptors);

		if constexpr (_DEBUG) {
			//GTSL::StaticString<128> name("Pipeline layout: "); name += layoutName.GetString();
			//pipelineLayout.Name = name;
		}

		GAL::PushConstant pushConstant;
		pushConstant.Stage = setLayoutData.Stage;
		pushConstant.NumberOf4ByteSlots = 32;
		setLayoutData.PipelineLayout.Initialize(renderSystem->GetRenderDevice(), &pushConstant, bindingsSetLayouts);

		return SetLayoutHandle(hash);
	}

	struct SubSetInfo
	{
		SubSetType Type;
		SubSetHandle* Handle;
		uint32 Count;
	};

	SetLayoutHandle AddSetLayout(RenderSystem* renderSystem, SetLayoutHandle parent, const GTSL::Range<SubSetInfo*> subsets)
	{
		GTSL::Array<SubSetDescriptor, 16> subSetInfos;
		for (auto e : subsets) { subSetInfos.EmplaceBack(e.Type, e.Count); }
		return AddSetLayout(renderSystem, parent, subSetInfos);
	}

	SetHandle AddSet(RenderSystem* renderSystem, Id setName, SetLayoutHandle setLayoutHandle, const GTSL::Range<SubSetInfo*> setInfo) {
		GTSL::Array<BindingsSetLayout::BindingDescriptor, 16> bindingDescriptors;

		for (auto& ss : setInfo) {
			GAL::ShaderStage enabledShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::COMPUTE;

			switch (ss.Type)
			{
			case SubSetType::BUFFER: {
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::STORAGE_BUFFER, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}
			case SubSetType::READ_TEXTURES: {
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::COMBINED_IMAGE_SAMPLER, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}
			case SubSetType::WRITE_TEXTURES: {
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::STORAGE_IMAGE, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}
			case SubSetType::RENDER_ATTACHMENT: {
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::INPUT_ATTACHMENT, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}
			case SubSetType::ACCELERATION_STRUCTURE: {
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::ACCELERATION_STRUCTURE, enabledShaderStages, ss.Count, GAL::BindingFlag() });
				break;
			}
			}
		}

		auto setHandle = makeSetEx(renderSystem, setName, setLayoutHandle, bindingDescriptors);

		auto& set = sets[setHandle()];

		uint32 i = 0;

		for (auto& ss : setInfo) {
			*ss.Handle = SubSetHandle({ setHandle, i, bindingDescriptors[i].BindingType });
			++i;
		}

		return setHandle;
	}

	//[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, GTSL::Range<MemberInfo*> members) {
	//	GAL::BufferUse bufferUses, notBufferFlags;
	//
	//	auto bufferIndex = buffers.Emplace(); auto& bufferData = buffers[bufferIndex];
	//
	//	auto parseMembers = [&](auto&& self, GTSL::Range<MemberInfo*> levelMembers, uint16 level) -> uint32 {
	//		uint32 offset = 0;
	//
	//		for (uint8 m = 0; m < levelMembers.ElementCount(); ++m) {
	//			if (levelMembers[m].Type == Member::DataType::PAD) { offset += levelMembers[m].Count; continue; }
	//
	//			auto memberDataIndex = bufferData.MemberData.GetLength();
	//			auto& member = bufferData.MemberData.EmplaceBack();
	//
	//			member.ByteOffsetIntoStruct = offset;
	//			member.Level = level;
	//			member.Type = levelMembers[m].Type;
	//			member.Count = levelMembers[m].Count;
	//
	//			*static_cast<MemberHandle<byte>*>(levelMembers[m].Handle) = MemberHandle<byte>(bufferIndex, memberDataIndex);
	//
	//			if (levelMembers[m].Type == Member::DataType::STRUCT) {
	//				member.Size = self(self, levelMembers[m].MemberInfos, level + 1);
	//			}
	//			else {
	//				if (levelMembers[m].Type == Member::DataType::SHADER_HANDLE) {
	//					bufferUses |= GAL::BufferUses::SHADER_BINDING_TABLE;
	//					notBufferFlags |= GAL::BufferUses::ACCELERATION_STRUCTURE; notBufferFlags |= GAL::BufferUses::STORAGE;
	//				}
	//
	//				member.Size = dataTypeSize(levelMembers[m].Type);
	//			}
	//
	//			offset += member.Size * member.Count;
	//		}
	//
	//		return offset;
	//	};
	//
	//	uint32 bufferSize = parseMembers(parseMembers, members, 0);
	//
	//	if (bufferSize != 0) {
	//		if constexpr (_DEBUG) {
	//			GTSL::StaticString<64> name("Buffer");
	//			//createInfo.Name = name;
	//		}
	//
	//		bufferUses |= GAL::BufferUses::ADDRESS; bufferUses |= GAL::BufferUses::STORAGE;
	//
	//		for (uint8 f = 0; f < queuedFrames; ++f) {
	//			renderSystem->AllocateScratchBufferMemory(bufferSize, bufferUses & ~notBufferFlags, &bufferData.Buffers[f], &bufferData.RenderAllocations[f]);
	//			bufferData.Size[f] = bufferSize;
	//		}
	//	}
	//
	//	return BufferHandle(bufferIndex);
	//}

	//[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, MemberInfo member) {
	//	return CreateBuffer(renderSystem, GTSL::Range<MemberInfo*>(1, &member));
	//}

	struct BindingsSetData
	{
		BindingsSetLayout BindingsSetLayout;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		uint32 DataSize = 0;
	};

	/**
	 * \brief Updates the iterator hierarchy level to index the specified member.
	 * \param iterator BufferIterator object to update.
	 * \param member MemberHandle that refers to the struct that we want the iterator to point to.
	 */
	//void UpdateIteratorMember(BufferIterator& iterator, MemberHandle<void*> member, const uint32 index = 0)
	//{
	//	//static_assert(T == (void*), "Type can only be struct!");
	//
	//	auto& bufferData = buffers[member.BufferIndex]; auto& memberData = bufferData.MemberData[member.MemberIndirectionIndex];
	//
	//	for (uint32 i = iterator.Levels.GetLength(); i < memberData.Level + 1; ++i) {
	//		iterator.Levels.EmplaceBack(0);
	//	}
	//
	//	for (uint32 i = iterator.Levels.GetLength(); i > memberData.Level + 1; --i) {
	//		iterator.Levels.PopBack();
	//	}
	//
	//	int32 shiftedElements = index - iterator.Levels.back();
	//
	//	iterator.Levels.back() = index;
	//
	//	iterator.ByteOffset += shiftedElements * memberData.Size;
	//
	//	bufferData.Written[frame] = true;
	//}

	void CopyWrittenBuffers(RenderSystem* renderSystem) {
		for (auto& e : buffers) {
			if (e.Written[renderSystem->GetCurrentFrame()]) {
			}
			else {
				auto beforeFrame = uint8(renderSystem->GetCurrentFrame() - uint8(1)) % renderSystem->GetPipelinedFrames();
				if (e.Written[beforeFrame]) {
					GTSL::MemCopy(e.Size[renderSystem->GetCurrentFrame()], e.RenderAllocations[beforeFrame].Data, e.RenderAllocations[renderSystem->GetCurrentFrame()].Data);
				}
			}

			e.Written[renderSystem->GetCurrentFrame()] = false;
		}
	}

private:
	inline static const Id RENDER_TASK_NAME{ u8"RenderOrchestrator::Render" };
	inline static const Id SETUP_TASK_NAME{ u8"RenderOrchestrator::Setup" };
	inline static const Id CLASS_NAME{ u8"RenderOrchestrator" };

	inline static constexpr uint32 RENDER_DATA_BUFFER_SIZE = 262144;
	inline static constexpr uint32 RENDER_DATA_BUFFER_SLACK_SIZE = 4096;
	inline static constexpr uint32 RENDER_DATA_BUFFER_PAGE_SIZE = RENDER_DATA_BUFFER_SIZE + RENDER_DATA_BUFFER_SLACK_SIZE;
	
	void onRenderEnable(GameInstance* gameInstance, const GTSL::Range<const TaskDependency*> dependencies);
	void onRenderDisable(GameInstance* gameInstance);

	GTSL::HashMap<uint64, GTSL::Vector<LayerHandle, BE::PAR>, BE::PAR> nodesByName;
	
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
		uint8 streamsCount = 0, buffersCount = 0;

		//IndexStreamHandle AddIndexStream() {			
		//	++indecesCount;
		//	return indexStreams.EmplaceBack(IndexStreamHandle(streamsCount++));
		//}

		//void UpdateIndexStream(IndexStreamHandle indexStreamHandle, CommandList commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 value);
		
		//void PopIndexStream(IndexStreamHandle indexStreamHandle) {
		//	--streamsCount; --indecesCount;
		//	BE_ASSERT(indexStreamHandle() == streamsCount);
		//}

		DataStreamHandle AddDataStream() {
			++buffersCount;
			return DataStreamHandle(streamsCount++);
		}
		
		void PopData(DataStreamHandle dataStreamHandle) {
			--streamsCount; --buffersCount;
			BE_ASSERT(dataStreamHandle() == streamsCount);
		}
	};
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 32>, BE::PersistentAllocatorReference> setupSystemsAccesses;
	
	GTSL::HashMap<Id, SystemHandle, BE::PersistentAllocatorReference> renderManagers;

	struct RenderDataBuffer {
		BufferHandle BufferHandle;
		GTSL::Array<uint32, 16> Elements;
	};
	GTSL::Array<RenderDataBuffer, 32> renderBuffers;
	
	Id resultAttachment;
	
	LayerHandle sceneRenderPass, globalData, cameraDataLayer;

	void transitionImages(CommandList commandBuffer, RenderSystem* renderSystem, const InternalLayer& internal_layer);

	struct ShaderLoadInfo
	{
		ShaderLoadInfo() = default;
		ShaderLoadInfo(const BE::PAR& allocator) noexcept : Buffer(allocator), Component(0) {}
		ShaderLoadInfo(ShaderLoadInfo&& other) noexcept : Buffer(MoveRef(other.Buffer)), Component(other.Component) {}
		GTSL::Buffer<BE::PAR> Buffer; uint32 Component;
	};

	void onShaderInfosLoaded(TaskInfo taskInfo, ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo shaderInfos, ShaderLoadInfo shaderLoadInfo);
	void onShadersLoaded(TaskInfo taskInfo, ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo);

	/// <summary>
	/// Keeps all public rendering nodes. Their positions are fixed.
	/// </summary>
	GTSL::Vector<PublicLayer, BE::PAR> renderingTree;

	/// <summary>
	/// Stores all internal rendering nodes. Elements in this array change positions depending on the order in which things will be rendered for best performance.
	/// </summary>
	GTSL::FixedVector<InternalLayer, BE::PAR> internalRenderingTree;

	GTSL::StaticMap<Id, InternalLayerHandle, 16> renderPasses;
	GTSL::Array<InternalLayerHandle, 16> renderPassesInOrder;

	GTSL::Extent2D sizeHistory[MAX_CONCURRENT_FRAMES];
	
	//MATERIAL STUFF
	struct RayTracingPipelineData {
		struct ShaderGroupData {
			uint32 RoundedEntrySize = 0;
			BufferHandle Buffer;

			MemberHandle<void*> EntryHandle;
			MemberHandle<GAL::ShaderHandle> ShaderHandle;
			MemberHandle<GAL::DeviceAddress> BufferBufferReferencesMemberHandle;
			
			//GTSL::Vector<ShaderRegisterData, BE::PAR> Shaders;
		} ShaderGroups[4];

		Pipeline Pipeline;
	};
	GTSL::FixedVector<RayTracingPipelineData, BE::PAR> rayTracingPipelines;

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

	struct MaterialInstance
	{
		Id Name;
		uint8 Counter = 0, Target = 0;
		Pipeline Pipeline;
	};
	
	struct MaterialData {
		Id Name;
		GTSL::Vector<MaterialInstance, BE::PAR> MaterialInstances;
		GTSL::StaticMap<Id, MemberHandle<uint32>, 16> ParametersHandles;
		GTSL::Array<ShaderResourceManager::Parameter, 16> Parameters;
		BufferHandle BufferHandle;

		MaterialData(const BE::PAR& allocator) : MaterialInstances(2, allocator) {}
	};
	GTSL::FixedVector<MaterialData, BE::PAR> materials;
	GTSL::HashMap<Id, uint32, BE::PAR> materialsByName;

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

	GTSL::HashMap<Id, uint32, BE::PersistentAllocatorReference> texturesRefTable;

	GTSL::Vector<uint32, BE::PAR> latestLoadedTextures;
	GTSL::FixedVector<GTSL::Vector<MaterialInstanceHandle, BE::PAR>, BE::PersistentAllocatorReference> pendingMaterialsPerTexture;
	
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
	DynamicTaskHandle<ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo, ShaderLoadInfo> onShaderInfosLoadHandle;
	DynamicTaskHandle<ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*>, ShaderLoadInfo> onShaderGroupLoadHandle;

	[[nodiscard]] const RenderPass* getAPIRenderPass(const Id renderPassName) const {
		return &getLayer(renderPasses.At(renderPassName)).RenderPass.APIRenderPass.RenderPass;
	}
	
	[[nodiscard]] uint8 getAPISubPassIndex(const Id renderPass) const {
		return getLayer(renderPasses.At(renderPass)).RenderPass.APIRenderPass.APISubPass;
	}

	uint32 dataTypeSize(Member::DataType data)
	{
		switch (data)
		{
		case Member::DataType::FLOAT32: return 4;
		case Member::DataType::UINT32: return 4;
		case Member::DataType::UINT64: return 8;
		case Member::DataType::MATRIX4: return 4 * 4 * 4;
		case Member::DataType::MATRIX3X4: return 4 * 3 * 4;
		case Member::DataType::FVEC4: return 4 * 4;
		case Member::DataType::INT32: return 4;
		case Member::DataType::FVEC2: return 4 * 2;
		case Member::DataType::SHADER_HANDLE: {
			if constexpr (API == GAL::RenderAPI::VULKAN) { return 32; } //aligned size
		}
		default: BE_ASSERT(false, "Unknown value!")
		}
	}

	void updateDescriptors(TaskInfo taskInfo) {
		auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>(u8"RenderSystem");

		for (auto& e : queuedSetUpdates) {
			resizeSet(renderSystem, e);
		}

		queuedSetUpdates.Clear();

		auto& descriptorsUpdate = descriptorsUpdates[renderSystem->GetCurrentFrame()];

		for (auto& set : descriptorsUpdate.sets) {
			Vector<BindingsPool::BindingsUpdateInfo, BE::TAR> bindingsUpdateInfos(16/*bindings sets*/, GetTransientAllocator());

			for (auto& subSet : set.GetElements()) {
				for (auto& b : subSet) {
					for (auto& a : b.GetElements()) {
						BindingsPool::BindingsUpdateInfo bindingsUpdateInfo;
						bindingsUpdateInfo.Type = sets[set.First].SubSets[b.First].Type;
						bindingsUpdateInfo.BindingsSet = &sets[set.First].BindingsSet[renderSystem->GetCurrentFrame()];
						bindingsUpdateInfo.SubsetIndex = b.First;

						for (auto& t : a) {
							bindingsUpdateInfo.BindingIndex = t.First;
							bindingsUpdateInfo.BindingUpdateInfos = t.GetElements();
							bindingsUpdateInfos.EmplaceBack(bindingsUpdateInfo);
						}
					}
				}

				sets[set.First].BindingsPool[renderSystem->GetCurrentFrame()].Update(renderSystem->GetRenderDevice(), bindingsUpdateInfos, GetTransientAllocator());
			}
		}

		descriptorsUpdate.Reset();
	}

	static constexpr GAL::BindingType BUFFER_BINDING_TYPE = GAL::BindingType::STORAGE_BUFFER;

	void updateSubBindingsCount(SubSetHandle subSetHandle, uint32 newCount) {
		auto& set = sets[subSetHandle().SetHandle()];
		auto& subSet = set.SubSets[subSetHandle().Subset];

		RenderSystem* renderSystem;

		if (subSet.AllocatedBindings < newCount)
		{
			BE_ASSERT(false, "OOOO");
		}
	}

	struct BufferData {
		RenderAllocation RenderAllocations[MAX_CONCURRENT_FRAMES];
		GPUBuffer Buffers[MAX_CONCURRENT_FRAMES];
		GTSL::Bitfield<128> WrittenAreas[MAX_CONCURRENT_FRAMES];
		bool Written[MAX_CONCURRENT_FRAMES]{ false };
		uint32 Size[MAX_CONCURRENT_FRAMES]{ 0 };

		struct MemberData {
			uint16 ByteOffsetIntoStruct;
			uint16 Count = 0;
			uint8 Level = 0;
			Member::DataType Type;
			uint16 Size;
		};
		GTSL::Array<MemberData, 16> MemberData;
	};
	GTSL::FixedVector<BufferData, BE::PAR> buffers;

	struct DescriptorsUpdate
	{
		DescriptorsUpdate(const BE::PAR& allocator) : sets(16, allocator) {
		}

		void AddBufferUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsPool::BufferBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void AddTextureUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsPool::TextureBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void AddAccelerationStructureUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsPool::AccelerationStructureBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void Reset()
		{
			sets.Clear();
		}

		GTSL::SparseVector<GTSL::SparseVector<GTSL::SparseVector<BindingsPool::BindingUpdateInfo, BE::PAR>, BE::PAR>, BE::PAR> sets;

	private:
		void addUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsPool::BindingUpdateInfo update)
		{
			if (sets.IsSlotOccupied(subSetHandle().SetHandle())) {
				auto& set = sets[subSetHandle().SetHandle()];

				if (set.IsSlotOccupied(subSetHandle().Subset)) {
					auto& subSet = set[subSetHandle().Subset];

					if (subSet.IsSlotOccupied(binding)) {
						subSet[binding] = update;
					} else { //there isn't binding
						subSet.EmplaceAt(binding, update);
					}
				} else {//there isn't sub set
					auto& subSet = set.EmplaceAt(subSetHandle().Subset, 32, sets.GetAllocator());
					//subSet.First = bindingType;
					subSet.EmplaceAt(binding, update);
				}
			} else { //there isn't set
				auto& set = sets.EmplaceAt(subSetHandle().SetHandle(), 16, sets.GetAllocator());

				auto& subSet = set.EmplaceAt(subSetHandle().Subset, 32, sets.GetAllocator());				
				subSet.EmplaceAt(binding, update);
			}
		}
	};

	GTSL::Array<DescriptorsUpdate, MAX_CONCURRENT_FRAMES> descriptorsUpdates;

	/**
	 * \brief Stores all data per binding set.
	 */
	struct SetData {
		Id Name;
		//SetHandle Parent;
		uint32 Level = 0;
		PipelineLayout PipelineLayout;
		BindingsSetLayout BindingsSetLayout;
		BindingsPool BindingsPool[MAX_CONCURRENT_FRAMES];
		BindingsSet BindingsSet[MAX_CONCURRENT_FRAMES];

		/**
		 * \brief Stores all data per sub set, and manages managed buffers.
		 * Each struct instance is pointed to by one binding. But a big per sub set buffer is used to store all instances.
		 */
		struct SubSetData {
			GAL::BindingType Type;
			uint32 AllocatedBindings = 0;
		};
		GTSL::Array<SubSetData, 16> SubSets;
	};
	GTSL::FixedVector<SetData, BE::PAR> sets;

	GTSL::PagedVector<SetHandle, BE::PAR> queuedSetUpdates;

	struct SetLayoutData {
		uint8 Level = 0;

		SetLayoutHandle Parent;
		BindingsSetLayout BindingsSetLayout;
		PipelineLayout PipelineLayout;
		GAL::ShaderStage Stage;
	};
	GTSL::HashMap<uint64, SetLayoutData, BE::PAR> setLayoutDatas;

	SetHandle makeSetEx(RenderSystem* renderSystem, Id setName, SetLayoutHandle setLayoutHandle, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDescriptors) {
		auto setHandle = SetHandle(sets.Emplace());
		auto& set = sets[setHandle()];

		auto& setLayout = setLayoutDatas[setLayoutHandle()];

		set.Level = setLayout.Level;
		set.BindingsSetLayout = setLayout.BindingsSetLayout;
		set.PipelineLayout = setLayout.PipelineLayout;

		if (bindingDescriptors.ElementCount()) {
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> name(u8"Bindings pool. Set: "); name += setName.GetString();
				//bindingsPoolCreateInfo.Name = name;
			}

			GTSL::Array<BindingsPool::BindingsPoolSize, 10> bindingsPoolSizes;

			for (auto e : bindingDescriptors) {
				bindingsPoolSizes.PushBack(BindingsPool::BindingsPoolSize{ e.BindingType, e.BindingsCount * renderSystem->GetPipelinedFrames() });
				set.SubSets.EmplaceBack(); auto& subSet = set.SubSets.back();
				subSet.Type = e.BindingType;
				subSet.AllocatedBindings = e.BindingsCount;
			}


			for (uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
				if constexpr (_DEBUG) {
					GTSL::StaticString<64> name(u8"BindingsSet. Set: "); name += setName.GetString();
				}

				set.BindingsPool[f].Initialize(renderSystem->GetRenderDevice(), bindingsPoolSizes, 1);
				set.BindingsSet[f].Initialize(renderSystem->GetRenderDevice(), set.BindingsPool[f], setLayout.BindingsSetLayout);
			}
		}

		return setHandle;
	}

	void resizeSet(RenderSystem* renderSystem, SetHandle setHandle) {

	}
};

class StaticMeshRenderManager : public RenderManager
{
public:
	StaticMeshRenderManager(const InitializeInfo& initializeInfo);
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}

	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;

private:
	RenderOrchestrator::MemberHandle<void*> staticMeshStruct;
	RenderOrchestrator::MemberHandle<GTSL::Matrix4> matrixUniformBufferMemberHandle;
	RenderOrchestrator::MemberHandle<GAL::DeviceAddress> vertexBufferReferenceHandle, indexBufferReferenceHandle;
	RenderOrchestrator::MemberHandle<uint32> materialInstance;
	RenderOrchestrator::LayerHandle staticMeshRenderGroup;
	BufferHandle bufferHandle;
	RenderOrchestrator::MemberHandle<void*> staticMeshInstanceDataStruct;

	struct Mesh {
		RenderOrchestrator::LayerHandle LayerHandle;
		StaticMeshHandle StaticMeshHandle;
	};
	GTSL::Vector<Mesh, BE::PAR> meshes;
};

class UIRenderManager : public RenderManager
{
public:
	UIRenderManager(const InitializeInfo& initializeInfo) : RenderManager(initializeInfo, u8"UIRenderManager") {
		auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>(u8"RenderSystem");
		auto* renderOrchestrator = initializeInfo.GameInstance->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");

		//RenderOrchestrator::CreateMaterialInfo createMaterialInfo;
		//createMaterialInfo.RenderSystem = renderSystem;
		//createMaterialInfo.GameInstance = initializeInfo.GameInstance;
		//createMaterialInfo.MaterialName = "UIMat";
		//createMaterialInfo.InstanceName = "UIMat";
		//createMaterialInfo.ShaderResourceManager = BE::Application::Get()->GetResourceManager<ShaderResourceManager>("ShaderResourceManager");
		//createMaterialInfo.TextureResourceManager = BE::Application::Get()->GetResourceManager<TextureResourceManager>("TextureResourceManager");
		//uiMaterial = renderOrchestrator->CreateMaterial(createMaterialInfo);
		//
		//square = renderSystem->CreateMesh("BE_UI_SQUARE", 0, GetUIMaterial());
		//renderSystem->UpdateMesh(square, 4, 4 * 2, 6, 2, GTSL::Array<GAL::ShaderDataType, 4>{ GAL::ShaderDataType::FLOAT2 });
		////
		//auto* meshPointer = renderSystem->GetMeshPointer(square);
		//GTSL::MemCopy(4 * 2 * 4, SQUARE_VERTICES, meshPointer);
		//meshPointer += 4 * 2 * 4;
		//GTSL::MemCopy(6 * 2, SQUARE_INDICES, meshPointer);
		//renderSystem->UpdateMesh(square);
		//renderSystem->SetWillWriteMesh(square, false);	
		//
		//GTSL::Array<MaterialSystem::MemberInfo, 8> members;
		//members.EmplaceBack(&matrixUniformBufferMemberHandle, 1);
		////members.EmplaceBack(4); //padding
		//
		////TODO: MAKE A CORRECT PATH FOR DECLARING RENDER PASSES
		//
		//auto bufferHandle = materialSystem->CreateBuffer(renderSystem, MaterialSystem::MemberInfo(&uiDataStruct, 16, members));
		//materialSystem->BindBufferToName(bufferHandle, "UIRenderGroup");
		//renderOrchestrator->AddToRenderPass("UIRenderPass", "UIRenderGroup");
	}
	
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}

	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;
	RenderSystem::MeshHandle GetSquareMesh() const { return square; }
	MaterialInstanceHandle GetUIMaterial() const { return uiMaterial; }

private:
	RenderSystem::MeshHandle square;

	RenderOrchestrator::MemberHandle<GTSL::Matrix4> matrixUniformBufferMemberHandle, colorHandle;
	RenderOrchestrator::MemberHandle<void*> uiDataStruct;

	uint8 comps = 2;
	MaterialInstanceHandle uiMaterial;
};
