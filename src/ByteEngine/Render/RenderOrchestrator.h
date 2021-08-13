#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Vector.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/PagedVector.h>
#include <GTSL/SparseVector.hpp>
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
class RenderGroup;
struct TaskInfo;

class RenderManager : public System
{
public:
	RenderManager(const InitializeInfo& initializeInfo, const char8_t* name) : System(initializeInfo, name) {}
	
	virtual void GetSetupAccesses(GTSL::StaticVector<TaskDependency, 16>& dependencies) = 0;

	struct SetupInfo
	{
		ApplicationManager* GameInstance;
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
	
	enum class NodeType : uint8 {
		DISPATCH, RAY_TRACE, MATERIAL, MESHES, RENDER_PASS, LAYER
	};

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
	
	MAKE_HANDLE(uint32, Node);

	struct DataKey {
		uint32 Offset;
		MemberHandle<void*> Member;
	};

protected:
	enum class InternalNodeType {
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

	MAKE_HANDLE(uint32, InternalNode)
	
	struct InternalNode {
		InternalNodeType Type;
		uint16 DirectChildren = 0, IndirectChildren = 0;
		uint32 Offset = ~0U;
		GTSL::ShortString<32> Name;
		bool Enabled = true;

		InternalNodeHandle Next;

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
			uint32 PipelineIndex = 0;
		};
		
		struct RenderPassData {
			PassType Type;
			GTSL::StaticVector<AttachmentData, 8> Attachments;
			GAL::PipelineStage PipelineStages;
			MemberHandle<uint32> RenderTargetReferences;
			
			RenderPassData() : Type(PassType::RASTER), Attachments(), PipelineStages(), APIRenderPass() {
			}
			
			union {
				APIRenderPassData APIRenderPass;
			};
		};

		struct LayerData {
			RenderSystem::BufferHandle BufferHandle;
		};
		
		union {
			MaterialData Material;
			MeshData Mesh;
			RenderPassData RenderPass;
			DispatchData Dispatch;
			RayTraceData RayTrace;
			LayerData Node;
		};

		InternalNode(const InternalNodeType t) : Type(t) {
			switch (Type) {
			case InternalNodeType::DISPATCH: break;
			case InternalNodeType::RAY_TRACE: break;
			case InternalNodeType::MATERIAL: break;
			case InternalNodeType::MESH: break;
			case InternalNodeType::RENDER_PASS: ::new(&RenderPass) RenderPassData(); break;
			case InternalNodeType::LAYER: break;
			case InternalNodeType::MATERIAL_INSTANCE: break;
			default: ;
			}
		}
		
		~InternalNode() {			
			switch (Type) {
			case InternalNodeType::DISPATCH: GTSL::Destroy(Dispatch); break;
			case InternalNodeType::RAY_TRACE: GTSL::Destroy(RayTrace); break;
			case InternalNodeType::MATERIAL: GTSL::Destroy(Material); break;
			case InternalNodeType::MESH: GTSL::Destroy(Mesh); break;
			case InternalNodeType::RENDER_PASS: GTSL::Destroy(RenderPass); break;
			case InternalNodeType::LAYER: GTSL::Destroy(Node); break;
			default: ;
			}
		}
	};
	
	struct PublicNode {
		NodeType Type; uint8 Level = 0;
		Id Name;
		uint32 Offset = ~0U;

		NodeHandle Parent;
		uint32 Children = 0;
		uint32 InstanceCount = 0;

		InternalNodeHandle EndOfChain;
		
		GTSL::StaticMap<uint64, NodeHandle, 8> ChildrenMap;

		struct InternalNodeData {
			InternalNodeHandle InternalNode;
			GTSL::StaticMap<uint64, InternalNodeHandle, 8> ChildrenMap;
		};
		GTSL::StaticVector<InternalNodeData, 8> InternalSiblings;
	};	

	PublicNode& getNode(const NodeHandle NodeHandle) {
		return renderingTree[NodeHandle()];
	}
	
	InternalNode& getNode(const InternalNodeHandle internal_layer_handle) {
		return internalRenderingTree[internal_layer_handle()];
	}

	const InternalNode& getNode(const InternalNodeHandle internal_layer_handle) const {
		return internalRenderingTree[internal_layer_handle()];
	}

	InternalNode& getNode2(NodeHandle layer_handle) {
		return internalRenderingTree[renderingTree[layer_handle()].InternalSiblings.front().InternalNode()];
	}

public:	
	struct MemberInfo : Member
	{
		MemberInfo() = default;
		MemberInfo(const uint32 count) : Member(count, Member::DataType::PAD) {}
		MemberInfo(MemberHandle<uint32>* memberHandle, const uint32 count = 1) : Member(count, DataType::UINT32), Handle(memberHandle) {}
		MemberInfo(MemberHandle<uint64>* memberHandle, const uint32 count = 1) : Member(count, DataType::UINT64), Handle(memberHandle) {}
		MemberInfo(MemberHandle<float32>* memberHandle, const uint32 count = 1) : Member(count, DataType::FLOAT32), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GAL::DeviceAddress>* memberHandle, const uint32 count = 1) : Member(count, DataType::UINT64), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GTSL::Matrix4>* memberHandle, const uint32 count = 1) : Member(count, DataType::MATRIX4), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GTSL::Matrix3x4>* memberHandle, const uint32 count = 1) : Member(count, DataType::MATRIX3X4), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GAL::ShaderHandle>* memberHandle, const uint32 count = 1) : Member(count, DataType::SHADER_HANDLE), Handle(memberHandle) {}
		MemberInfo(MemberHandle<void*>* memberHandle, const uint32 count, GTSL::Range<MemberInfo*> memberInfos, const uint32 alignment = 0) : Member(count, DataType::STRUCT), Handle(memberHandle), MemberInfos(memberInfos), alignment(alignment) {}

		void* Handle = nullptr;
		GTSL::Range<MemberInfo*> MemberInfos;
		uint16 alignment = 0;
	};

private:	
	InternalNode& addInternalLayer(const uint64 key, NodeHandle publicSiblingHandle, NodeHandle publicParentHandle, InternalNodeType type) {
		InternalNodeHandle nodeHandle;
		InternalNode* layer = nullptr;
		
		if (publicParentHandle) {
			auto& publicParent = getNode(publicParentHandle);
			
			auto internalParentHandle = publicParent.InternalSiblings.back().InternalNode;
			auto& internalParent = getNode(internalParentHandle);
			internalParent.DirectChildren++;
			
			if (auto search = publicParent.InternalSiblings.back().ChildrenMap.TryGet(key)) {
				nodeHandle = search.Get();

				NodeHandle p = publicParentHandle;

				while (getNode(p).Parent) {
					for(auto& e : getNode(p).InternalSiblings) {
						++getNode(e.InternalNode).IndirectChildren;
					}
					
					p = getNode(p).Parent;
				}
				
				return getNode(nodeHandle);
			}			
			
			uint32 insertPosition = internalRenderingTree.Emplace(type);
			nodeHandle = InternalNodeHandle(insertPosition);
			layer = &internalRenderingTree[insertPosition];

			NodeHandle p = publicParentHandle;

			while (getNode(p).Parent) {
				for (auto& e : getNode(p).InternalSiblings) {
					++getNode(e.InternalNode).IndirectChildren;
				}
				
				p = getNode(p).Parent;
			}
			
			if (!internalParent.Next) {
				internalParent.Next = nodeHandle;
				getNode(p).EndOfChain = nodeHandle;
			} else {
				getNode(getNode(p).EndOfChain).Next = nodeHandle;
			}
			
			publicParent.InternalSiblings.back().ChildrenMap.Emplace(key, nodeHandle);
			
			auto& sibling = getNode(publicSiblingHandle).InternalSiblings.EmplaceBack();
			sibling.InternalNode = nodeHandle;			
		} else {
			uint32 insertPosition = internalRenderingTree.Emplace(type);
			nodeHandle = InternalNodeHandle(insertPosition);
			layer = &internalRenderingTree[insertPosition];
			auto& sibling = getNode(publicSiblingHandle).InternalSiblings.EmplaceBack();
			sibling.InternalNode = nodeHandle;
		}

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

	MAKE_HANDLE(uint32, Buffer)
	MAKE_HANDLE(uint64, SetLayout)
	
	DataKey AddData(MemberHandle<void*> memberHandle) {
		auto offset = renderDataOffset;
		renderDataOffset += memberHandle.Size;
		return DataKey{ offset, memberHandle };
	}
	
	void AddData(NodeHandle layer_handle, DataKey data_key) {
		auto& publicNode = getNode(layer_handle);
		auto& privateNode = getNode(publicNode.InternalSiblings.front().InternalNode);
		publicNode.Offset = data_key.Offset;
		privateNode.Offset = data_key.Offset;
	}
	
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo);

	void AddRenderManager(ApplicationManager* gameInstance, const Id renderManager, const SystemHandle systemReference);
	void RemoveRenderManager(ApplicationManager* gameInstance, const Id renderGroupName, const SystemHandle systemReference);
	NodeHandle GetCameraDataLayer() const { return cameraDataNode; }

	uint32 renderDataOffset = 0;
	SetLayoutHandle globalSetLayout;
	SetHandle globalBindingsSet;
	RenderAllocation allocation;
	GPUBuffer buffer;
	NodeHandle rayTraceNode;

	struct CreateMaterialInfo
	{
		Id MaterialName, InstanceName;
		ShaderResourceManager* ShaderResourceManager = nullptr;
		ApplicationManager* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager;
	};
	[[nodiscard]] MaterialInstanceHandle CreateMaterial(const CreateMaterialInfo& info);

	void AddAttachment(Id name, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type, GTSL::RGBA clearColor);
	
	struct PassData {
		struct AttachmentReference {
			Id Name;
		};
		GTSL::StaticVector<AttachmentReference, 8> ReadAttachments, WriteAttachments;

		PassType PassType;
	};
	void AddPass(Id name, NodeHandle parent, RenderSystem* renderSystem, PassData passData);

	void OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize);

	/**
	 * \brief Enables or disables the rendering of a render pass
	 * \param renderPassName Name of the render Pass to toggle
	 * \param enable Whether to enable(true) or disable(false) the render pass
	 */
	void ToggleRenderPass(NodeHandle renderPassName, bool enable);

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

	void onPush(NodeHandle layer, PublicNode& publicLayer, uint8 level) {
		publicLayer.Level = level;
		publicLayer.InstanceCount++;
		publicLayer.Parent = NodeHandle();

		//for (uint32 i = layer(); i < internalIndirectionTable.GetLength(); ++i) {
		//	++internalIndirectionTable[i];
		//}
	}

	void onPush(NodeHandle layer, PublicNode& publicLayer, NodeHandle parent) {
		onPush(layer, publicLayer, getNode(parent).Level + 1);
		publicLayer.Parent = parent;

		NodeHandle p = parent;

		do {
			++getNode(p).Children;
			p = getNode(p).Parent;
		} while (p);
	}
	
	NodeHandle PushNode() {
		auto nodeHandle = NodeHandle(renderingTree.Emplace());
		auto& self = renderingTree[nodeHandle()];
		onPush(nodeHandle, self, 0);
		return nodeHandle;
	}
	
	NodeHandle PushNode(const NodeHandle parent) {
		auto nodeHandle = NodeHandle(renderingTree.Emplace());
		auto& self = renderingTree[nodeHandle()];
		onPush(nodeHandle, self, parent);
		return nodeHandle;
	}
	
	[[nodiscard]] NodeHandle AddNode(const uint64 key, NodeHandle parent, const NodeType layerType) {
		NodeHandle nodeHandle;
		
		if (parent) {			
			if (getNode(parent).ChildrenMap.Find(key)) {
				auto& pa = getNode(parent);		
				nodeHandle = pa.ChildrenMap.At(key);
				onPush(nodeHandle, getNode(nodeHandle), parent);
				return nodeHandle;
			}
			
			nodeHandle = PushNode(parent);
			getNode(parent).ChildrenMap.Emplace(key, nodeHandle);
			
		} else {
			nodeHandle = PushNode();
		}

		
		auto& data = getNode(nodeHandle);		
		data.Type = layerType;
		
		switch (data.Type) {
		case NodeType::DISPATCH: {
			addInternalLayer(key, nodeHandle, parent, InternalNodeType::DISPATCH);
			break;
		}
		case NodeType::RAY_TRACE: {
			addInternalLayer(key, nodeHandle, parent, InternalNodeType::RAY_TRACE);
			break;
		}
		case NodeType::MATERIAL: {
			break;
		}
		case NodeType::MESHES: {
			break;
		}
		case NodeType::RENDER_PASS: {
			auto& layer = addInternalLayer(key, nodeHandle, parent, InternalNodeType::RENDER_PASS);

			if(layer.RenderPass.Type == PassType::RAY_TRACING) {
				layer.Enabled = false;
			}
			
			break;
		}
		case NodeType::LAYER: {
			addInternalLayer(key, nodeHandle, parent, InternalNodeType::LAYER);
			break;
		}
		}
		
		//nodesByName.Emplace((uint64)data.Level << 60 | key, nodeHandle);
		nodesByName.Emplace((uint64)layerType << 60 | key, nodeHandle);
		
		return nodeHandle;
	}

	[[nodiscard]] NodeHandle AddNode(const Id name, const NodeHandle parent, const NodeType layerType) {
		auto l = AddNode(name(), parent, layerType);
		auto& t = getNode(l);
		t.Name = name; getNode(t.InternalSiblings.back().InternalNode).Name = name.GetString();
		return l;
	}
	
	NodeHandle AddMaterial(NodeHandle parentHandle, MaterialInstanceHandle materialHandle) {
		auto materialKey = (uint64)materialHandle.MaterialInstanceIndex << 32 | materialHandle.MaterialIndex;
		
		auto layer = AddNode(materialKey, parentHandle, NodeType::MATERIAL);

		auto& material = addInternalLayer(materialKey, layer, parentHandle, InternalNodeType::MATERIAL);
		auto& materialInstance = addInternalLayer(materialHandle.MaterialInstanceIndex, layer, layer, InternalNodeType::MATERIAL_INSTANCE);
		
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
	
	NodeHandle AddMesh(NodeHandle parentNodeHandle, RenderSystem::MeshHandle meshHandle, GTSL::Range<const GAL::ShaderDataType*> meshVertexLayout, MemberHandle<void*> handle) {
		auto layer = AddNode(meshHandle(), parentNodeHandle, NodeType::MESHES);

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
		
		auto& meshNode = addInternalLayer(meshHandle(), layer, parentNodeHandle, InternalNodeType::MESH);
		
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

	struct BufferWriteKey {
		RenderSystem::BufferHandle Handle;
		uint32 Offset = 0, LastSize = 0, Counter = 0;

		operator uint32() const { return Counter; }

		template<typename T>
		BufferWriteKey(RenderSystem::BufferHandle buffer_handle, const MemberHandle<T> member_handle, uint32 offset) : Handle(buffer_handle), Offset(offset), LastSize(member_handle.Size) {
			
		}
		
		void operator()(const MemberHandle<void*> member_handle) {
			Offset = member_handle.Offset;
		}

		void operator++() {
			Offset += LastSize;
			++Counter;
		}
	};
	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const BufferHandle buffer_handle, MemberHandle<void*> member_handle) {
		render_system->SignalBufferWrite(buffers[buffer_handle()].BufferHandle);
		return BufferWriteKey(buffers[buffer_handle()].BufferHandle, member_handle, 0);
	}

	template<typename T>
	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const NodeHandle node_handle, MemberHandle<T> member_handle) {
		render_system->SignalBufferWrite(renderBuffers[0].BufferHandle);
		return BufferWriteKey(renderBuffers[0].BufferHandle, member_handle, getNode(node_handle).Offset);
	}

	template<typename T>
	void Write(RenderSystem* renderSystem, BufferWriteKey buffer_write_key, MemberHandle<T> member, const T& data) {
		*reinterpret_cast<T*>(renderSystem->GetBufferPointer(buffer_write_key.Handle) + buffer_write_key.Offset + member.Offset) = data;
	}
	
	auto GetSceneRenderPass() const { return sceneRenderPass; }
	
	void WriteBinding(RenderSystem* render_system, SubSetHandle subSetHandle, uint32 bindingIndex, AccelerationStructure accelerationStructure) {
		for (uint8 f = 0; f < render_system->GetPipelinedFrames(); ++f) {
			descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
		}
	}

	void WriteBinding(SubSetHandle subSetHandle, uint32 bindingIndex, AccelerationStructure accelerationStructure, uint8 f) {
		descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
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

		GTSL::StaticVector<BindingsSetLayout, 16> bindingsSetLayouts;

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

		GTSL::StaticVector<BindingsSetLayout::BindingDescriptor, 10> subSetDescriptors;

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

			subSetDescriptors.EmplaceBack(BindingsSetLayout::BindingDescriptor{ bindingType, shaderStage, e.BindingsCount, bindingFlags });
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
		GTSL::StaticVector<SubSetDescriptor, 16> subSetInfos;
		for (auto e : subsets) { subSetInfos.EmplaceBack(e.Type, e.Count); }
		return AddSetLayout(renderSystem, parent, subSetInfos);
	}

	SetHandle AddSet(RenderSystem* renderSystem, Id setName, SetLayoutHandle setLayoutHandle, const GTSL::Range<SubSetInfo*> setInfo) {
		GTSL::StaticVector<BindingsSetLayout::BindingDescriptor, 16> bindingDescriptors;

		for (auto& ss : setInfo) {
			GAL::ShaderStage enabledShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::COMPUTE;

			switch (ss.Type)
			{
			case SubSetType::BUFFER: {
				bindingDescriptors.EmplaceBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::STORAGE_BUFFER, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}
			case SubSetType::READ_TEXTURES: {
				bindingDescriptors.EmplaceBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::COMBINED_IMAGE_SAMPLER, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}
			case SubSetType::WRITE_TEXTURES: {
				bindingDescriptors.EmplaceBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::STORAGE_IMAGE, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}
			case SubSetType::RENDER_ATTACHMENT: {
				bindingDescriptors.EmplaceBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::INPUT_ATTACHMENT, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}
			case SubSetType::ACCELERATION_STRUCTURE: {
				bindingDescriptors.EmplaceBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::ACCELERATION_STRUCTURE, enabledShaderStages, ss.Count, GAL::BindingFlag() });
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

	[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, GTSL::Range<MemberInfo*> members) {
		GAL::BufferUse bufferUses, notBufferFlags;
	
		auto bufferIndex = buffers.Emplace(); auto& bufferData = buffers[bufferIndex];
	
		auto parseMembers = [&](auto&& self, GTSL::Range<MemberInfo*> levelMembers, uint16 level) -> uint32 {
			uint32 offset = 0;
	
			for (uint8 m = 0; m < levelMembers.ElementCount(); ++m) {
				if (levelMembers[m].Type == Member::DataType::PAD) { offset += levelMembers[m].Count; continue; }
	
				auto memberDataIndex = bufferData.MemberData.GetLength();
				auto& member = bufferData.MemberData.EmplaceBack();
	
				member.ByteOffsetIntoStruct = GTSL::Math::RoundUpByPowerOf2(offset, static_cast<uint32>(levelMembers[m].alignment));
				member.Type = levelMembers[m].Type;
				member.Count = levelMembers[m].Count;
	
				*static_cast<MemberHandle<byte>*>(levelMembers[m].Handle) = MemberHandle<byte>(bufferIndex, memberDataIndex);
	
				if (levelMembers[m].Type == Member::DataType::STRUCT) {
					member.Size = self(self, levelMembers[m].MemberInfos, level + 1);
				}
				else {
					if (levelMembers[m].Type == Member::DataType::SHADER_HANDLE) {
						bufferUses |= GAL::BufferUses::SHADER_BINDING_TABLE;
						notBufferFlags |= GAL::BufferUses::ACCELERATION_STRUCTURE; notBufferFlags |= GAL::BufferUses::STORAGE;
					}
	
					member.Size = dataTypeSize(levelMembers[m].Type);
				}
	
				offset += GTSL::Math::RoundUpByPowerOf2(member.Size * member.Count, static_cast<uint32>(levelMembers[m].alignment));
			}
	
			return offset;
		};
	
		uint32 bufferSize = parseMembers(parseMembers, members, 0);
	
		if (bufferSize != 0) {
			bufferData.BufferHandle = renderSystem->CreateBuffer(bufferSize, bufferUses & ~notBufferFlags, true, false);
		}
		
		return BufferHandle(bufferIndex);
	}

	void AddMesh(RenderSystem* render_system, const NodeHandle parent_handle, const RenderSystem::MeshHandle mesh_handle, const MaterialInstanceHandle material_instance_handle, DataKey data_key) {
		auto rtmi = rayTracingMaterials[getMaterialHash(material_instance_handle)];

		auto& material = materials[material_instance_handle.MaterialIndex];
		
		auto& rt = rayTracingPipelines[rtmi.pipeline];
		auto& sg = rt.ShaderGroups[rtmi.sg];
		
		auto swk = GetBufferWriteKey(render_system, sg.Buffer, sg.ShaderEntryMemberHandle[sg.ObjectCount++]);
		Write(render_system, swk, sg.MaterialDataHandle, render_system->GetBufferDeviceAddress(renderBuffers[0].BufferHandle) + material.DataKey.Offset); //get material data
		Write(render_system, swk, sg.ObjectDataHandle, render_system->GetBufferDeviceAddress(renderBuffers[0].BufferHandle) + data_key.Offset); //get object data
	}
	
	//[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, MemberInfo member) {
	//	return CreateBuffer(renderSystem, GTSL::Range<MemberInfo*>(1, &member));
	//}	
	struct BindingsSetData {
		BindingsSetLayout BindingsSetLayout;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		uint32 DataSize = 0;
	};

	void SetNodeState(const NodeHandle layer_handle, const bool state) {
		getNode2(layer_handle).Enabled = state;
	}

	NodeHandle GetSceneReference() const {
		return NodeHandle(); //todo: gen
	}

private:
	inline static const Id RENDER_TASK_NAME{ u8"RenderOrchestrator::Render" };
	inline static const Id SETUP_TASK_NAME{ u8"RenderOrchestrator::Setup" };
	inline static const Id CLASS_NAME{ u8"RenderOrchestrator" };

	inline static constexpr uint32 RENDER_DATA_BUFFER_SIZE = 262144;
	inline static constexpr uint32 RENDER_DATA_BUFFER_SLACK_SIZE = 4096;
	inline static constexpr uint32 RENDER_DATA_BUFFER_PAGE_SIZE = RENDER_DATA_BUFFER_SIZE + RENDER_DATA_BUFFER_SLACK_SIZE;
	
	void onRenderEnable(ApplicationManager* gameInstance, const GTSL::Range<const TaskDependency*> dependencies);
	void onRenderDisable(ApplicationManager* gameInstance);

	GTSL::HashMap<uint64, NodeHandle, BE::PAR> nodesByName; //todo: fix, how?
	
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

	GTSL::StaticVector<GTSL::StaticVector<GAL::ShaderDataType, 24>, 32> vertexLayouts;
	
	struct RenderState {
		uint8 APISubPass = 0, MaxAPIPass = 0;
		GAL::ShaderStage ShaderStages;
		uint8 streamsCount = 0, buffersCount = 0;

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
	GTSL::Vector<GTSL::StaticVector<TaskDependency, 32>, BE::PersistentAllocatorReference> setupSystemsAccesses;
	
	GTSL::HashMap<Id, SystemHandle, BE::PersistentAllocatorReference> renderManagers;

	struct RenderDataBuffer {
		RenderSystem::BufferHandle BufferHandle;
		GTSL::StaticVector<uint32, 16> Elements;
	};
	GTSL::StaticVector<RenderDataBuffer, 32> renderBuffers;
	
	Id resultAttachment;
	
	NodeHandle sceneRenderPass, globalData, cameraDataNode;

	void transitionImages(CommandList commandBuffer, RenderSystem* renderSystem, const InternalNode& internal_layer);

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
	GTSL::FixedVector<PublicNode, BE::PAR> renderingTree;

	/// <summary>
	/// Stores all internal rendering nodes. Elements in this array change positions depending on the order in which things will be rendered for best performance.
	/// </summary>
	GTSL::FixedVector<InternalNode, BE::PAR> internalRenderingTree;

	GTSL::StaticMap<Id, InternalNodeHandle, 16> renderPasses;
	GTSL::StaticVector<InternalNodeHandle, 16> renderPassesInOrder;

	GTSL::Extent2D sizeHistory[MAX_CONCURRENT_FRAMES];
	
	//MATERIAL STUFF
	struct RayTracingPipelineData {
		struct ShaderGroupData {
			uint32 RoundedEntrySize = 0;
			BufferHandle Buffer;
			uint32 ShaderCount = 0;

			uint32 ObjectCount = 0;
			
			MemberHandle<void*> ShaderGroupDataHandle;
				MemberHandle<GAL::ShaderHandle> ShaderHandle;
				MemberHandle<void*> ShaderEntryMemberHandle;
					MemberHandle<GAL::DeviceAddress> MaterialDataHandle, ObjectDataHandle;

			//GTSL::Vector<ShaderRegisterData, BE::PAR> Shaders;
		} ShaderGroups[4];

		uint32 ResourceCounter = 0, Target = 0;
		
		Pipeline Pipeline;
	};
	GTSL::FixedVector<RayTracingPipelineData, BE::PAR> rayTracingPipelines;

	static uint64 getMaterialHash(const MaterialInstanceHandle material_instance_handle) {
		return (uint64)material_instance_handle.MaterialInstanceIndex << 32 | material_instance_handle.MaterialIndex;
	}
	
	struct RTMI {
		uint32 pipeline = 0, sg = 0, instance = 0;
	};
	GTSL::StaticMap<uint64, RTMI, 16> rayTracingMaterials;
	
	uint32 textureIndex = 0, imageIndex = 0;
	
	struct CreateTextureInfo
	{
		Id TextureName;
		ApplicationManager* GameInstance = nullptr;
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

	struct MaterialInstance {
		Id Name;
		uint32 Counter = 0, Target = 0;
	};
	
	struct MaterialData {
		Id Name;
		GTSL::Vector<MaterialInstance, BE::PAR> MaterialInstances;
		GTSL::StaticMap<Id, MemberHandle<uint32>, 16> ParametersHandles;
		GTSL::StaticVector<ShaderResourceManager::Parameter, 16> Parameters;
		DataKey DataKey;

		MaterialData(const BE::PAR& allocator) : MaterialInstances(2, allocator) {}
	};
	GTSL::FixedVector<MaterialData, BE::PAR> materials;

	struct RasterMaterialData{
		RasterMaterialData(const BE::PAR& allocator) : Instances(allocator) {}
		
		GTSL::Vector<Pipeline, BE::PAR> Instances;
	};
	GTSL::FixedVector<RasterMaterialData, BE::PAR> rasterMaterials;
	
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
	GTSL::StaticMap<Id, Attachment, 32> attachments{ 16 };

	void updateImage(Attachment& attachment, GAL::TextureLayout textureLayout, GAL::PipelineStage stages, GAL::AccessType writeAccess) {
		attachment.Layout = textureLayout; attachment.ConsumingStages = stages; attachment.AccessType = writeAccess;
	}

	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureInfoLoadHandle;
	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureLoadHandle;
	DynamicTaskHandle<ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo, ShaderLoadInfo> onShaderInfosLoadHandle;
	DynamicTaskHandle<ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*>, ShaderLoadInfo> onShaderGroupLoadHandle;

	[[nodiscard]] const RenderPass* getAPIRenderPass(const Id renderPassName) const {
		return &getNode(renderPasses.At(renderPassName)).RenderPass.APIRenderPass.RenderPass;
	}
	
	[[nodiscard]] uint8 getAPISubPassIndex(const Id renderPass) const {
		return getNode(renderPasses.At(renderPass)).RenderPass.APIRenderPass.APISubPass;
	}

	uint32 dataTypeSize(Member::DataType data)
	{
		switch (data) {
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
		default: __debugbreak(); return 0;
		}
	}

	void updateDescriptors(TaskInfo taskInfo) {
		auto* renderSystem = taskInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");

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
		RenderSystem::BufferHandle BufferHandle;

		struct MemberData {
			uint32 ByteOffsetIntoStruct;
			uint32 Count = 0;
			Member::DataType Type;
			uint16 Size;
		};
		GTSL::StaticVector<MemberData, 16> MemberData;
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

	GTSL::StaticVector<DescriptorsUpdate, MAX_CONCURRENT_FRAMES> descriptorsUpdates;

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
		GTSL::StaticVector<SubSetData, 16> SubSets;
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

			GTSL::StaticVector<BindingsPool::BindingsPoolSize, 10> bindingsPoolSizes;

			for (auto e : bindingDescriptors) {
				bindingsPoolSizes.EmplaceBack(BindingsPool::BindingsPoolSize{ e.BindingType, e.BindingsCount * renderSystem->GetPipelinedFrames() });
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

	NodeHandle getNodeByName(const MaterialInstanceHandle material_instance_handle) {
		return nodesByName[(uint64)NodeType::MATERIAL << 60 | getMaterialHash(material_instance_handle)];
	}
};

class StaticMeshRenderManager : public RenderManager
{
public:
	StaticMeshRenderManager(const InitializeInfo& initializeInfo);
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}

	void GetSetupAccesses(GTSL::StaticVector<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;

private:
	RenderOrchestrator::MemberHandle<void*> staticMeshStruct;
	RenderOrchestrator::MemberHandle<GTSL::Matrix4> matrixUniformBufferMemberHandle;
	RenderOrchestrator::MemberHandle<GAL::DeviceAddress> vertexBufferReferenceHandle, indexBufferReferenceHandle;
	RenderOrchestrator::MemberHandle<uint32> materialInstance;
	RenderOrchestrator::NodeHandle staticMeshRenderGroup;
	RenderOrchestrator::BufferHandle bufferHandle;
	RenderOrchestrator::MemberHandle<void*> staticMeshInstanceDataStruct;

	struct Mesh {
		RenderOrchestrator::NodeHandle NodeHandle;
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
		//createMaterialInfo.ApplicationManager = initializeInfo.ApplicationManager;
		//createMaterialInfo.MaterialName = "UIMat";
		//createMaterialInfo.InstanceName = "UIMat";
		//createMaterialInfo.ShaderResourceManager = BE::Application::Get()->GetResourceManager<ShaderResourceManager>("ShaderResourceManager");
		//createMaterialInfo.TextureResourceManager = BE::Application::Get()->GetResourceManager<TextureResourceManager>("TextureResourceManager");
		//uiMaterial = renderOrchestrator->CreateMaterial(createMaterialInfo);
		//
		//square = renderSystem->CreateMesh("BE_UI_SQUARE", 0, GetUIMaterial());
		//renderSystem->UpdateMesh(square, 4, 4 * 2, 6, 2, GTSL::StaticVector<GAL::ShaderDataType, 4>{ GAL::ShaderDataType::FLOAT2 });
		////
		//auto* meshPointer = renderSystem->GetMeshPointer(square);
		//GTSL::MemCopy(4 * 2 * 4, SQUARE_VERTICES, meshPointer);
		//meshPointer += 4 * 2 * 4;
		//GTSL::MemCopy(6 * 2, SQUARE_INDICES, meshPointer);
		//renderSystem->UpdateMesh(square);
		//renderSystem->SetWillWriteMesh(square, false);	
		//
		//GTSL::StaticVector<MaterialSystem::MemberInfo, 8> members;
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

	void GetSetupAccesses(GTSL::StaticVector<TaskDependency, 16>& dependencies) override;

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
