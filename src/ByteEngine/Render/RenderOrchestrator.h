#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Vector.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/PagedVector.h>
#include <GTSL/SparseVector.hpp>

#include "ByteEngine/Id.h"
#include "RenderSystem.h"
#include "RenderTypes.h"
#include "StaticMeshRenderGroup.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/Tasks.h"
#include "ByteEngine/Resources/ShaderResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"
#include "GTSL/Tree.hpp"

class RenderOrchestrator;
class RenderState;
class RenderGroup;
struct TaskInfo;

//Data Entry
//	- Data on a globally accesible buffer

//Make Member
//	- Make a struct declaration

//Add Node
//	- Adds a node to the render tree

//Make Data Ker
//	- Adds a member allocation to the global buffer

//Bind Data Key
//	- Bind a data key to a node

class RenderManager : public System
{
public:
	RenderManager(const InitializeInfo& initializeInfo, const char8_t* name) : System(initializeInfo, name) {}

	struct SetupInfo {
		ApplicationManager* GameInstance;
		RenderSystem* RenderSystem;
		//RenderState* RenderState;
		GTSL::Matrix4 ViewMatrix, ProjectionMatrix;
		RenderOrchestrator* RenderOrchestrator;
	};
};

class RenderOrchestrator : public System {
public:
	enum class PassType : uint8 {
		RASTER, COMPUTE, RAY_TRACING
	};
	
	enum class NodeType : uint8 {
		DISPATCH, RAY_TRACE, MATERIAL, MESHES, RENDER_PASS, LAYER
	};

	struct Member {
		Member() = default;
		Member(const uint32 count, const Id type) : Count(count), Type(type) {}

		uint32 Count = 1;
		Id Type;
	};

	struct MemberHandle {
		MemberHandle() = default;
		MemberHandle(const Id id, uint32 off, uint32 s) : Hash(id), Offset(off), Size(s) {}
		MemberHandle(const Id name) : Hash(name) {}

		Id Hash; uint32 Offset = 0, Size = 0;

		MemberHandle operator[](const uint32 index) {
			return MemberHandle{ Hash, Offset + Size * index, Size };
		}
	};
	
	struct NodeHandle {
		NodeHandle() = default;
		NodeHandle(const uint32 val) : value(val) {}

		uint32 operator()() const { return value; }

		operator bool() const { return value; }
	private:
		uint32 value = 0;
	};

	MAKE_HANDLE(uint32, DataKey);

protected:
	enum class InternalNodeType {
		DISPATCH, RAY_TRACE, MATERIAL, MESH, RENDER_PASS, LAYER, MATERIAL_INSTANCE
	};

	MAKE_HANDLE(uint32, InternalNode);
	MAKE_HANDLE(uint64, Resource);

	struct AttachmentData {
		Id Name;
		GAL::TextureLayout Layout;
		GAL::PipelineStage ConsumingStages;
		GAL::AccessType Access;
	};

	struct APIRenderPassData {
		FrameBuffer FrameBuffer[MAX_CONCURRENT_FRAMES];
		RenderPass RenderPass;
		uint8 APISubPass = 0, SubPassCount = 0;
	};

public:	
	struct MemberInfo : Member {
		MemberInfo() = default;
		MemberInfo(const uint32 count) : Member(count, Id(u8"pad")) {}
		MemberInfo(MemberHandle* memberHandle, const uint32 count, Id type) : Member(count, type), Handle(memberHandle) {}
		MemberInfo(MemberHandle* memberHandle, const uint32 count, GTSL::Range<MemberInfo*> memberInfos, Id type, const uint32 alignment = 0) : Member(count, type), Handle(memberHandle), MemberInfos(memberInfos), alignment(alignment) {}

		MemberHandle* Handle = nullptr;
		GTSL::Range<MemberInfo*> MemberInfos;
		uint16 alignment = 0;
	};

	explicit RenderOrchestrator(const InitializeInfo& initializeInfo);

	MAKE_HANDLE(uint32, Set);

	struct SubSetDescription {
		SetHandle SetHandle; uint32 Subset;
		GAL::BindingType Type;
	};

	MAKE_HANDLE(SubSetDescription, SubSet)

	MAKE_HANDLE(uint32, Buffer)
	MAKE_HANDLE(uint64, SetLayout)
	
	DataKeyHandle MakeDataKey() {
		auto pos = dataKeys.GetLength();
		dataKeys.EmplaceBack(0xFFFFFFFF);
		return DataKeyHandle(pos);
	}

	DataKeyHandle MakeDataKey(MemberHandle memberHandle) {
		auto offset = renderDataOffset;
		renderDataOffset += memberHandle.Size;
		auto pos = dataKeys.GetLength();
		dataKeys.EmplaceBack(offset);
		return DataKeyHandle(pos);
	}

	void BindDataKey(NodeHandle layer_handle, DataKeyHandle data_key) {
		auto& privateNode = getInternalNodeFromPublicHandle(layer_handle); //BUG: CHECK WHICH NODE TO RETRIEVE, EG: FRONT/BACK
		privateNode.Offset = dataKeys[data_key()];
	}

	void BindDataKey(InternalNodeHandle layer_handle, DataKeyHandle data_key) {
		auto& privateNode = getInternalNode(layer_handle);
		privateNode.Offset = dataKeys[data_key()];
	}
	
	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo, RenderSystem* renderSystem);

	//HACKS, REMOVE
	NodeHandle GetGlobalDataLayer() const { return globalData; }
	NodeHandle GetCameraDataLayer() const { return cameraDataNode; }
	NodeHandle GetSceneRenderPass() const { return renderPasses[u8"SceneRenderPass"].First; }
	//HACKS, REMOVE

	struct CreateMaterialInfo {
		Id MaterialName, InstanceName;
		ShaderResourceManager* ShaderResourceManager = nullptr;
		ApplicationManager* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager;
	};
	[[nodiscard]] MaterialInstanceHandle CreateMaterial(const CreateMaterialInfo& info);

	void AddAttachment(Id attachmentName, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type, GTSL::RGBA clearColor);
	
	struct PassData {
		struct AttachmentReference {
			Id Name;
		};
		GTSL::StaticVector<AttachmentReference, 8> ReadAttachments, WriteAttachments;

		PassType PassType;
	};
	NodeHandle AddPass(GTSL::StringView renderPassName, NodeHandle parent, RenderSystem* renderSystem, PassData passData, ApplicationManager* am, ShaderResourceManager* srm);

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

	MemberHandle MakeMember(Id structName, const GTSL::Range<MemberInfo*> members) {
		GAL::BufferUse bufferUses, notBufferFlags;
		
		auto parseMembers = [&](auto&& self, GTSL::Range<MemberInfo*> levelMembers, uint16 level) -> uint32 {
			uint32 size = 0, offset = 0;

			for (uint8 m = 0; m < levelMembers.ElementCount(); ++m) {
				if (levelMembers[m].Type == u8"pad") { offset += levelMembers[m].Count; continue; }

				if (levelMembers[m].MemberInfos.ElementCount()) {
					size = self(self, levelMembers[m].MemberInfos, level + 1);
				} else {
					if (levelMembers[m].Type == u8"ShaderHandle") {
						bufferUses |= GAL::BufferUses::SHADER_BINDING_TABLE;
						notBufferFlags |= GAL::BufferUses::ACCELERATION_STRUCTURE; notBufferFlags |= GAL::BufferUses::STORAGE;
					}

					size = dataTypeSize(levelMembers[m].Type);
				}
				
				*levelMembers[m].Handle = MemberHandle{ levelMembers[m].Type, offset, size };

				offset += size * levelMembers[m].Count;
			}

			return offset;
		};

		uint32 bufferSize = parseMembers(parseMembers, members, 0);
		
		//for(auto e : members) {
		//	hash |= static_cast<GTSL::UnderlyingType<decltype(e.Type)>>(e.Type);
		//	hash |= e.Count << 8;
		//}

		return MemberHandle{ structName, 0, bufferSize };
	}
	
	NodeHandle AddMaterial(NodeHandle parentHandle, MaterialInstanceHandle materialHandle) {
		auto materialKey = (uint64)materialHandle.MaterialInstanceIndex << 32 | materialHandle.MaterialIndex;
		
		auto layer = addNode(materialKey, parentHandle, NodeType::MATERIAL);

		auto& material = materials[materialHandle.MaterialIndex];

		auto materialNodeHandle = addInternalNode<MaterialInstanceData>(222, layer, parentHandle, InternalNodeType::MATERIAL);
		auto materialInstanceNodeHandle = addInternalNode<MaterialInstanceData>(material.PipelineStart, layer, parentHandle, InternalNodeType::MATERIAL_INSTANCE);

		//nodesByName.Emplace((uint64)InternalNodeType::MATERIAL_INSTANCE << 60 | material.PipelineStart, materialInstanceNodeHandle);

		auto& materialNode = getInternalNode(materialNodeHandle);
		auto& materialInstance = getInternalNode(materialInstanceNodeHandle);

		bindResourceToNode(materialNodeHandle, pipelines[material.PipelineStart].ResourceHandle);

		materialNode.Name = GTSL::StringView(materials[materialHandle.MaterialIndex].Name);
		//material.Boink.MaterialHandle = materialHandle;

		getPrivateNode<MaterialInstanceData>(materialInstanceNodeHandle).MaterialHandle = materialHandle;

		if constexpr (_DEBUG) {
			materialInstance.Name = GTSL::StaticString<64>(u8"Material Instance #");
			ToString(materialInstance.Name, materialHandle.MaterialInstanceIndex);
		}
		
		return layer;
	}

	NodeHandle AddLayer(Id layerName, NodeHandle parent) {
		auto publicNodeHandle = addNode(layerName, parent, NodeType::LAYER);
		auto internalNodeHandle = addInternalNode<LayerData>(layerName(), publicNodeHandle, parent, InternalNodeType::LAYER);
		getInternalNode(internalNodeHandle).Name = GTSL::StringView(layerName);
		return publicNodeHandle;
	}

	NodeHandle AddMesh(const RenderSystem::MeshHandle handle, const NodeHandle parentNodeHandle) {
		auto publicNodeHandle = addNode(handle(), parentNodeHandle, NodeType::MESHES);
		auto internalNodeHandle = addInternalNode<MeshData>(handle(), publicNodeHandle, parentNodeHandle, InternalNodeType::MESH);
		SetNodeState(internalNodeHandle, false);
		return publicNodeHandle;
	}

	void AddMesh(NodeHandle node_handle, RenderSystem::MeshHandle meshHandle, GTSL::Range<const GAL::ShaderDataType*> meshVertexLayout, MemberHandle handle) {
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

		getPrivateNodeFromPublicHandle<MeshData>(node_handle).Handle = meshHandle;

		SetNodeState(getInternalNodeHandleFromPublicHandle(node_handle), true);
	}

	struct BufferWriteKey {
		RenderSystem::BufferHandle Handle;
		uint32 Offset = 0, LastSize = 0, Counter = 0;

		operator uint32() const { return Counter; }

		BufferWriteKey(RenderSystem::BufferHandle buffer_handle, const MemberHandle member_handle, uint32 offset) : Handle(buffer_handle), Offset(offset), LastSize(member_handle.Size) {
			
		}
		
		void operator()(const MemberHandle member_handle) {
			Offset = member_handle.Offset;
		}

		void operator++() {
			Offset += LastSize;
			++Counter;
		}
	};

	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const ResourceHandle resource_handle, MemberHandle member_handle) {
		render_system->SignalBufferWrite(renderBuffers[0].BufferHandle);
		return BufferWriteKey(renderBuffers[0].BufferHandle, member_handle, getDataKeyOffset(resourceNodes[resource_handle()].Data));
	}

	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const BufferHandle buffer_handle, MemberHandle member_handle) {
		render_system->SignalBufferWrite(buffers[buffer_handle()].BufferHandle);
		return BufferWriteKey(buffers[buffer_handle()].BufferHandle, member_handle, 0);
	}

	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const NodeHandle node_handle, MemberHandle member_handle) {
		render_system->SignalBufferWrite(renderBuffers[0].BufferHandle);
		return BufferWriteKey(renderBuffers[0].BufferHandle, member_handle, getInternalNodeFromPublicHandle(node_handle).Offset);
	}

	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const InternalNodeHandle node_handle, MemberHandle member_handle) {
		render_system->SignalBufferWrite(renderBuffers[0].BufferHandle);
		return BufferWriteKey(renderBuffers[0].BufferHandle, member_handle, getInternalNode(node_handle).Offset);
	}

	template<typename T>
	void Write(RenderSystem* renderSystem, BufferWriteKey buffer_write_key, MemberHandle member, const T& data) {
		*reinterpret_cast<T*>(renderSystem->GetBufferPointer(buffer_write_key.Handle) + buffer_write_key.Offset + member.Offset) = data;
	}
	
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
		} else {
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
	
	struct SubSetDescriptor {
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

	struct SubSetInfo {
		SubSetType Type;
		SubSetHandle* Handle;
		uint32 Count;
	};
	SetLayoutHandle AddSetLayout(RenderSystem* renderSystem, SetLayoutHandle parent, const GTSL::Range<SubSetInfo*> subsets) {
		GTSL::StaticVector<SubSetDescriptor, 16> subSetInfos;
		for (auto e : subsets) { subSetInfos.EmplaceBack(e.Type, e.Count); }
		return AddSetLayout(renderSystem, parent, subSetInfos);
	}

	SetHandle AddSet(RenderSystem* renderSystem, Id setName, SetLayoutHandle setLayoutHandle, const GTSL::Range<SubSetInfo*> setInfo) {
		GTSL::StaticVector<BindingsSetLayout::BindingDescriptor, 16> bindingDescriptors;

		for (auto& ss : setInfo) {
			GAL::ShaderStage enabledShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::COMPUTE;

			switch (ss.Type) {
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

	[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, MemberHandle member_handle) {
		GAL::BufferUse bufferUses, notBufferFlags;
	
		auto bufferIndex = buffers.Emplace(); auto& bufferData = buffers[bufferIndex];
	
		uint32 bufferSize = member_handle.Size;
	
		if (bufferSize) {
			bufferData.BufferHandle = renderSystem->CreateBuffer(bufferSize, bufferUses & ~notBufferFlags, true, false);
		}
		
		return BufferHandle(bufferIndex);
	}

	void AddMesh(RenderSystem* render_system, const NodeHandle parent_handle, const RenderSystem::MeshHandle mesh_handle, const MaterialInstanceHandle material_instance_handle, DataKeyHandle data_key) {
		auto rtmi = rayTracingMaterials[getMaterialHash(material_instance_handle)];

		auto& material = materials[material_instance_handle.MaterialIndex];
		
		auto& rt = rayTracingPipelines[rtmi.pipeline];
		auto& sg = rt.ShaderGroups[rtmi.sg];
		
		auto swk = GetBufferWriteKey(render_system, sg.Buffer, sg.ShaderEntryMemberHandle[sg.ObjectCount++]);
		Write(render_system, swk, sg.MaterialDataHandle, render_system->GetBufferDeviceAddress(renderBuffers[0].BufferHandle) + getDataKeyOffset(material.DataKey)); //get material data
		Write(render_system, swk, sg.ObjectDataHandle, render_system->GetBufferDeviceAddress(renderBuffers[0].BufferHandle) + getDataKeyOffset(data_key)); //get object data
	}

	struct BindingsSetData {
		BindingsSetLayout BindingsSetLayout;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		uint32 DataSize = 0;
	};

	void SetNodeState(const NodeHandle layer_handle, const bool state) { //TODO: DONT CHANGE STATE IF THERE ARE PENDING RESOURCES WHICH SHOULD IMPEDE ENABLING THE NODE
		SetNodeState(getInternalNodeHandleFromPublicHandle(layer_handle), state);
	}

	void SetNodeState(const InternalNodeHandle layer_handle, const bool state) {
		renderingTree.ToggleBranch(layer_handle(), state);
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
	
	bool renderingEnabled = false;

	uint32 renderDataOffset = 0;
	SetLayoutHandle globalSetLayout;
	SetHandle globalBindingsSet;
	RenderAllocation allocation;
	GPUBuffer buffer;
	NodeHandle rayTraceNode;

	SubSetHandle renderGroupsSubSet;
	SubSetHandle renderPassesSubSet;

	MemberHandle cameraMatricesHandle;
	BufferHandle cameraDataBuffer;
	BufferHandle globalDataBuffer;
	MemberHandle globalDataHandle;
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
		
		void PopData() {
			--streamsCount; --buffersCount;
		}
	};

	struct RenderDataBuffer {
		RenderSystem::BufferHandle BufferHandle;
		GTSL::StaticVector<uint32, 16> Elements;
	};
	GTSL::StaticVector<RenderDataBuffer, 32> renderBuffers;

	struct InternalNode {
		//InternalNodeType Type;
		uint32 Offset = ~0U;
		GTSL::ShortString<32> Name;
	};

	struct MeshData {
		RenderSystem::MeshHandle Handle;
		uint32 InstanceCount = 0;
	};

	struct MaterialInstanceData {
		MaterialInstanceHandle MaterialHandle;
		uint8 VertexLayoutIndex;
	};

	struct DispatchData {
		GTSL::Extent3D DispatchSize;
		uint32 pipelineIndex;
	};

	struct RayTraceData {
		uint32 PipelineIndex = 0;
	};

	struct RenderPassData {
		PassType Type;
		GTSL::StaticVector<AttachmentData, 4> Attachments;
		GAL::PipelineStage PipelineStages;
		MemberHandle RenderTargetReferences;
		ResourceHandle ResourceHandle;

		RenderPassData() : Type(PassType::RASTER), Attachments(), PipelineStages(), APIRenderPass() {
		}

		union {
			APIRenderPassData APIRenderPass;
		};
	};

	struct LayerData {
		RenderSystem::BufferHandle BufferHandle;
	};

	struct PublicNode {
		NodeType Type; uint8 Level = 0;
		Id Name;
		//uint32 Offset = ~0U;
		uint32 InstanceCount = 0;
	};

	[[nodiscard]] NodeHandle addNode(const uint64 key, NodeHandle parent, const NodeType layerType) {
		//TODO: if node with same key already exists under same parent, return said node

		//if (const auto e = nodesByName.TryGet(key); e) { return e.Get(); }

		NodeHandle nodeHandle = NodeHandle(renderingTree.EmplaceAlpha(parent()));

		auto& data = getNode(nodeHandle);
		data.Type = layerType;

		return nodeHandle;
	}

	[[nodiscard]] NodeHandle addNode(const Id nodeName, const NodeHandle parent, const NodeType layerType) {
		auto l = addNode(nodeName(), parent, layerType);
		auto& t = getNode(l);
		t.Name = nodeName;
		return l;
	}

	PublicNode& getNode(const NodeHandle nodeHandle) {
		return renderingTree.GetAlpha(nodeHandle());
	}

	InternalNode& getInternalNode(const InternalNodeHandle internal_node_handle) {
		return renderingTree.GetBeta(internal_node_handle());
	}

	template<class T>
	T& getPrivateNode(const InternalNodeHandle internal_node_handle) {
		return renderingTree.GetClass<T>(internal_node_handle());
	}

	//template<class N>
	//const InternalNode<N>& getNode(const InternalNodeHandle internal_node_handle) const {
	//	return renderingTree.At<InternalNode<N>>(internal_node_handle());
	//}

	InternalNode& getInternalNodeFromPublicHandle(NodeHandle node_handle) {
		return renderingTree.GetBeta(renderingTree.GetBetaHandleFromAlpha(node_handle(), 0xFFFFFFFF));
	}

	InternalNodeHandle getInternalNodeHandleFromPublicHandle(NodeHandle node_handle) {
		return InternalNodeHandle(renderingTree.GetBetaHandleFromAlpha(node_handle(), 0xFFFFFFFF));
	}

	template<class  T>
	T& getPrivateNodeFromPublicHandle(NodeHandle layer_handle) {
		return renderingTree.GetClass<T>(renderingTree.GetBetaHandleFromAlpha(layer_handle(), 0xFFFFFFFF));
	}

	Id resultAttachment;
	
	NodeHandle globalData, cameraDataNode;

	void transitionImages(CommandList commandBuffer, RenderSystem* renderSystem, const RenderPassData* internal_layer);

	struct ShaderLoadInfo {
		ShaderLoadInfo() = default;
		ShaderLoadInfo(const BE::PAR& allocator) noexcept : Buffer(allocator), PipelineStart(0), MaterialIndex(0) {}
		ShaderLoadInfo(ShaderLoadInfo&& other) noexcept : Buffer(MoveRef(other.Buffer)), PipelineStart(other.PipelineStart), MaterialIndex(other.MaterialIndex), handle(other.handle) {}
		GTSL::Buffer<BE::PAR> Buffer; uint32 PipelineStart, MaterialIndex;
		InternalNodeHandle handle;
	};

	uint64 resourceCounter = 0;

	ResourceHandle makeResource() {
		resourceNodes.Emplace(++resourceCounter);
		return ResourceHandle(resourceCounter);
	}

	void bindResourceToNode(InternalNodeHandle node_handle, ResourceHandle resource_handle) {
		auto& resource = resourceNodes[resource_handle()];
		
		resource.NodeHandles.EmplaceBack(node_handle);

		SetNodeState(node_handle, resource.isValid());

		if (resource.Data && getDataKeyOffset(resource.Data) != 0xFFFFFFFF) {
			SetNodeDataKey(node_handle, resource.Data);
		}
	}

	void addDependencyCount(const ResourceHandle resource_handle) {
		auto& resource = resourceNodes[resource_handle()];

		++resource.Target;

		bool enableValue = resource.isValid();

		for (auto e : resource.NodeHandles) {
			SetNodeState(e, enableValue);

			if (resource.Data && getDataKeyOffset(resource.Data) != 0xFFFFFFFF) {
				SetNodeDataKey(e, resource.Data);
			}
		}
	}

	void SetNodeDataKey(const InternalNodeHandle internal_node_handle, const DataKeyHandle data_key_handle) {
		renderingTree.GetBeta(internal_node_handle()).Offset = getDataKeyOffset(data_key_handle);
	}

	void signalDependencyToResource(ResourceHandle resource_handle) {
		if (resourceNodes.Find(resource_handle())) {
			auto& resource = resourceNodes[resource_handle()];

			++resource.Count;

			if (resource.isValid()) {
				for (const auto& e : resource.NodeHandles) {
					SetNodeState(e, true);

					if (resource.Data && getDataKeyOffset(resource.Data) != 0xFFFFFFFF) {
						SetNodeDataKey(e, resource.Data);
					}
				}

				resourceNodes.Remove(resource_handle());
			}
		}
		else {
			BE_LOG_WARNING(u8"Tried to enable resource which is nnot available.")
		}
	}

	void bindDataKey(const ResourceHandle resource_handle, const DataKeyHandle data_key) {
		auto& resource = resourceNodes[resource_handle()];

		resource.Data = data_key;

		bool enableValue = resource.isValid();

		for (auto e : resource.NodeHandles) {
			SetNodeState(e, enableValue);

			if (resource.Data && getDataKeyOffset(resource.Data) != 0xFFFFFFFF) {
				SetNodeDataKey(e, resource.Data);
			}
		}
	}

	struct ResourceData {
		GTSL::StaticVector<InternalNodeHandle, 8> NodeHandles;
		uint32 Count = 0, Target = 0;
		DataKeyHandle Data;

		bool isValid() const { return Count == Target; }
	};
	GTSL::HashMap<uint64, ResourceData, BE::PAR> resourceNodes;
	GTSL::Vector<uint32, BE::PAR> dataKeys;

	uint32 getDataKeyOffset(const DataKeyHandle data_key_handle) const {
		return dataKeys[data_key_handle()];
	}
	
	void UpdateDataKey(const ResourceHandle resource_handle, MemberHandle member_handle) {
		dataKeys[resourceNodes[resource_handle()].Data()] = renderDataOffset;
		renderDataOffset += member_handle.Size;
	}

	void onShaderInfosLoaded(TaskInfo taskInfo, ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo shaderInfos, ShaderLoadInfo shaderLoadInfo);
	
	void onShadersLoaded(TaskInfo taskInfo, ShaderResourceManager*, RenderSystem*, ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo);

	GTSL::AlphaBetaTree<BE::PAR, PublicNode, InternalNode, LayerData, MaterialInstanceData, RayTraceData, DispatchData, MeshData, RenderPassData> renderingTree;

	GTSL::StaticMap<Id, InternalNodeHandle, 16> pendingNodes;

	GTSL::StaticMap<Id, GTSL::Pair<NodeHandle, InternalNodeHandle>, 16> renderPasses;
	GTSL::StaticVector<InternalNodeHandle, 16> renderPassesInOrder;

	GTSL::Extent2D sizeHistory[MAX_CONCURRENT_FRAMES];

	struct Pipeline {
		Pipeline(const BE::PAR& allocator) {}

		::Pipeline pipeline;
		ResourceHandle ResourceHandle;
	};
	GTSL::FixedVector<Pipeline, BE::PAR> pipelines;

	//MATERIAL STUFF
	struct RayTracingPipelineData {
		struct ShaderGroupData {
			uint32 RoundedEntrySize = 0;
			BufferHandle Buffer;
			uint32 ShaderCount = 0;

			uint32 ObjectCount = 0;
			
			MemberHandle ShaderGroupDataHandle;
				MemberHandle ShaderHandle;
				MemberHandle ShaderEntryMemberHandle;
					MemberHandle MaterialDataHandle, ObjectDataHandle;

			//GTSL::Vector<ShaderRegisterData, BE::PAR> Shaders;
		} ShaderGroups[4];
		
		uint32 PipelineIndex;
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
	
	struct CreateTextureInfo {
		GTSL::ShortString<64> TextureName;
		ApplicationManager* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager = nullptr;
	};
	uint32 createTexture(const CreateTextureInfo& createTextureInfo);
	
	struct MaterialLoadInfo {
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
	};
	
	struct MaterialData {
		Id Name;
		GTSL::Vector<MaterialInstance, BE::PAR> MaterialInstances;
		GTSL::StaticMap<Id, MemberHandle, 16> ParametersHandles;
		GTSL::StaticVector<ShaderResourceManager::Parameter, 16> Parameters;
		DataKeyHandle DataKey;
		GTSL::uint32 PipelineStart;

		MaterialData(const BE::PAR& allocator) : MaterialInstances(2, allocator) {}
	};
	GTSL::FixedVector<MaterialData, BE::PAR> materials;
	
	GTSL::HashMap<Id, uint32, BE::PAR> materialsByName;

	struct TextureLoadInfo {
		TextureLoadInfo() = default;

		TextureLoadInfo(uint32 component, RenderAllocation renderAllocation) : Component(component), RenderAllocation(renderAllocation)
		{}

		uint32 Component;
		RenderAllocation RenderAllocation;
		RenderSystem::TextureHandle TextureHandle;
	};
	void onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem*, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);
	void onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem*, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);

	GTSL::HashMap<Id, uint32, BE::PersistentAllocatorReference> texturesRefTable;

	GTSL::FixedVector<GTSL::Vector<uint32, BE::PAR>, BE::PersistentAllocatorReference> pendingPipelinesPerTexture;

	void addPendingPipelineToTexture(uint32 texture, uint32 pipelineIndex) {
		addDependencyCount(pipelines[pipelineIndex].ResourceHandle);
		pendingPipelinesPerTexture[texture].EmplaceBack(pipelineIndex);
	}
	
	struct Attachment {
		RenderSystem::TextureHandle TextureHandle[MAX_CONCURRENT_FRAMES];

		Id Name;
		GAL::TextureUse Uses; GAL::TextureLayout Layout;
		GAL::PipelineStage ConsumingStages; GAL::AccessType AccessType;
		GTSL::RGBA ClearColor; GAL::FormatDescriptor FormatDescriptor;
		uint32 ImageIndex;
	};
	GTSL::HashMap<Id, Attachment, BE::PAR> attachments;

	void updateImage(Attachment& attachment, GAL::TextureLayout textureLayout, GAL::PipelineStage stages, GAL::AccessType writeAccess) {
		attachment.Layout = textureLayout; attachment.ConsumingStages = stages; attachment.AccessType = writeAccess;
	}

	DynamicTaskHandle<TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureInfoLoadHandle;
	DynamicTaskHandle<TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureLoadHandle;
	DynamicTaskHandle<ShaderResourceManager::ShaderGroupInfo, ShaderLoadInfo> onShaderInfosLoadHandle;
	DynamicTaskHandle<ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*>, ShaderLoadInfo> onShaderGroupLoadHandle;

	[[nodiscard]] const RenderPass* getAPIRenderPass(const Id renderPassName) {
		return &getPrivateNode<RenderPassData>(renderPasses.At(renderPassName).Second).APIRenderPass.RenderPass;
	}
	
	[[nodiscard]] uint8 getAPISubPassIndex(const Id renderPassName) {
		return getPrivateNode<RenderPassData>(renderPasses.At(renderPassName).Second).APIRenderPass.APISubPass;
	}

	uint16 dataTypeSize(Id type) {
		return sizes[type];
	}

	GTSL::HashMap<Id, uint8, BE::PAR> sizes;

	void updateDescriptors(TaskInfo taskInfo) {
		auto* renderSystem = taskInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");

		//for (auto& e : queuedSetUpdates) {
		//	resizeSet(renderSystem, e);
		//}

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

		if (subSet.AllocatedBindings < newCount) {
			BE_ASSERT(false, "OOOO");
		}
	}

	struct BufferData {
		RenderSystem::BufferHandle BufferHandle;

		struct MemberData {
			uint32 ByteOffsetIntoStruct;
			uint32 Count = 0;
			Id Type;
			uint16 Size;
		};
		GTSL::StaticVector<MemberData, 16> MemberData;
	};
	GTSL::FixedVector<BufferData, BE::PAR> buffers;

	struct DescriptorsUpdate {
		DescriptorsUpdate(const BE::PAR& allocator) : sets(16, allocator) {
		}

		void AddBufferUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsPool::BufferBindingUpdateInfo update) {
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void AddTextureUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsPool::TextureBindingUpdateInfo update) {
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void AddAccelerationStructureUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsPool::AccelerationStructureBindingUpdateInfo update) {
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void Reset() {
			sets.Clear();
		}

		GTSL::SparseVector<GTSL::SparseVector<GTSL::SparseVector<BindingsPool::BindingUpdateInfo, BE::PAR>, BE::PAR>, BE::PAR> sets;

	private:
		void addUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsPool::BindingUpdateInfo update) {
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

	GTSL::StaticVector<GAL::VulkanSampler, 16> samplers;

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
				//GTSL::StaticString<64> name(u8"Bindings pool. Set: "); name += GTSL::StringView(setName);
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
					//GTSL::StaticString<64> name(u8"BindingsSet. Set: "); name += GTSL::StringView(setName);
				}

				set.BindingsPool[f].Initialize(renderSystem->GetRenderDevice(), bindingsPoolSizes, 1);
				set.BindingsSet[f].Initialize(renderSystem->GetRenderDevice(), set.BindingsPool[f], setLayout.BindingsSetLayout);
			}
		}

		return setHandle;
	}

	template<typename T>
	InternalNodeHandle addInternalNode(const uint64 key, NodeHandle publicSiblingHandle, NodeHandle publicParentHandle, InternalNodeType type) {
		auto betaNodeHandle = renderingTree.EmplaceBeta<T>(key, publicParentHandle(), publicSiblingHandle());
		auto& node = renderingTree.GetBeta(betaNodeHandle.Get());
		//node.Type = type;
		return InternalNodeHandle(betaNodeHandle.Get());
	}

	//InternalNodeHandle getNodeByName(const uint32 pipelineIndex) {
	//	return nodesByName[(uint64)InternalNodeType::MATERIAL_INSTANCE << 60 | pipelineIndex];
	//}

#if BE_DEBUG
	GAL::PipelineStage pipelineStages;
#endif
};

class StaticMeshRenderManager : public RenderManager
{
public:
	StaticMeshRenderManager(const InitializeInfo& initializeInfo);

	DynamicTaskHandle<StaticMeshHandle, Id, MaterialInstanceHandle> OnAddMesh;
	DynamicTaskHandle<StaticMeshHandle> OnUpdateMesh;

private:
	RenderOrchestrator::MemberHandle staticMeshStruct;
	RenderOrchestrator::MemberHandle matrixUniformBufferMemberHandle;
	RenderOrchestrator::MemberHandle vertexBufferReferenceHandle, indexBufferReferenceHandle;
	RenderOrchestrator::MemberHandle materialInstance;
	RenderOrchestrator::NodeHandle staticMeshRenderGroup;
	RenderOrchestrator::BufferHandle bufferHandle;
	RenderOrchestrator::MemberHandle staticMeshInstanceDataStruct;

	bool rayTracing = false;

	struct Mesh {
		RenderOrchestrator::NodeHandle NodeHandle;
		MaterialInstanceHandle MaterialHandle;
	};
	GTSL::HashMap<StaticMeshHandle, Mesh, BE::PAR> meshes;

	struct Resource {
		RenderSystem::MeshHandle RenderMesh;
		GTSL::Range<byte*> Buffer;
		GTSL::StaticVector<StaticMeshHandle, 8> Meshes;
		bool Loaded = false;
	};
	GTSL::HashMap<Id, Resource, BE::PAR> resources;

	DynamicTaskHandle<StaticMeshResourceManager::StaticMeshInfo> onStaticMeshLoadHandle;
	DynamicTaskHandle<StaticMeshResourceManager::StaticMeshInfo> onStaticMeshInfoLoadHandle;
	//DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo> onStaticMeshInfoLoadHandle;

	void onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, RenderSystem* render_system, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo) {
		auto& res = resources[staticMeshInfo.Name];

		render_system->UpdateMesh(res.RenderMesh, staticMeshInfo.VertexCount, staticMeshInfo.VertexSize, staticMeshInfo.IndexCount, staticMeshInfo.IndexSize, staticMeshInfo.VertexDescriptor);

		auto renderMeshHandle = res.RenderMesh;

		res.Buffer = GTSL::Range<byte*>(render_system->GetMeshSize(renderMeshHandle), render_system->GetMeshPointer(renderMeshHandle));

		staticMeshResourceManager->LoadStaticMesh(taskInfo.ApplicationManager, staticMeshInfo, render_system->GetBufferSubDataAlignment(), res.Buffer, onStaticMeshLoadHandle);
	}

	void onStaticMeshLoaded(TaskInfo taskInfo, RenderSystem* render_system, StaticMeshRenderGroup* render_group, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo){
		auto& res = resources[staticMeshInfo.Name];

		render_system->SignalMeshDataUpdate(res.RenderMesh);
		if (rayTracing) { render_system->UpdateRayTraceMesh(res.RenderMesh); }

		for(const auto e : res.Meshes) {
			onMeshLoad(render_system, render_group, render_orchestrator, res.RenderMesh, e);
			//meshLoadInfo.RenderSystem->SetWillWriteMesh(meshLoadInfo.MeshHandle, false);
		}

		res.Loaded = true;
	}

	//BUG: WE HAVE AN IMPLICIT DEPENDENCY ON ORDERING OF TASK, AS WE REQUIRE onAddMesh TO BE RUN BEFORE updateMesh, THIS ORDERING IS NOT CURRENTLY GUARANTEED BY THE TASK SYSTEM

	void onAddMesh(TaskInfo task_info, StaticMeshResourceManager* static_mesh_resource_manager, RenderOrchestrator* render_orchestrator, RenderSystem* render_system, StaticMeshRenderGroup* static_mesh_render_group, StaticMeshHandle static_mesh_handle, Id resourceName, MaterialInstanceHandle material_instance_handle) {
		auto& mesh = meshes.Emplace(static_mesh_handle);

		auto res = resources.TryEmplace(resourceName);

		if (res) {
			static_mesh_resource_manager->LoadStaticMeshInfo(task_info.ApplicationManager, resourceName, onStaticMeshInfoLoadHandle);
			res.Get().RenderMesh = render_system->CreateMesh(resourceName);
		} else {
			if (res.Get().Loaded) {
				onMeshLoad(render_system, static_mesh_render_group, render_orchestrator, res.Get().RenderMesh, static_mesh_handle);
			}
		}

		auto materialLayer = render_orchestrator->AddMaterial(render_orchestrator->GetSceneRenderPass(), material_instance_handle);
		auto meshNode = render_orchestrator->AddMesh(res.Get().RenderMesh, materialLayer);
		auto dataKey = render_orchestrator->MakeDataKey(staticMeshInstanceDataStruct);
		render_orchestrator->BindDataKey(meshNode, dataKey);

		mesh.NodeHandle = meshNode;

		res.Get().Meshes.EmplaceBack(static_mesh_handle);
	}

	void onMeshLoad(RenderSystem* renderSystem, StaticMeshRenderGroup* renderGroup, RenderOrchestrator* renderOrchestrator, RenderSystem::MeshHandle mesh_handle, StaticMeshHandle static_mesh_handle) {
		auto& mesh = meshes[static_mesh_handle];

		auto key = renderOrchestrator->GetBufferWriteKey(renderSystem, mesh.NodeHandle, staticMeshInstanceDataStruct);
		renderOrchestrator->Write(renderSystem, key, matrixUniformBufferMemberHandle, renderGroup->GetMeshTransform(static_mesh_handle));
		renderOrchestrator->Write(renderSystem, key, vertexBufferReferenceHandle, renderSystem->GetVertexBufferAddress(mesh_handle));
		renderOrchestrator->Write(renderSystem, key, indexBufferReferenceHandle, renderSystem->GetIndexBufferAddress(mesh_handle));
		renderOrchestrator->Write(renderSystem, key, materialInstance, mesh.MaterialHandle.MaterialInstanceIndex);

		if (rayTracing) {
			//info.RenderOrchestrator->AddMesh(info.RenderSystem, info.RenderOrchestrator->GetSceneReference(), e.MeshHandle, materialHandle, dataKey);
		}

		renderOrchestrator->AddMesh(mesh.NodeHandle, mesh_handle, renderSystem->GetMeshVertexLayout(mesh_handle), staticMeshInstanceDataStruct);
	}

	void updateMesh(TaskInfo, RenderSystem* renderSystem, StaticMeshRenderGroup* renderGroup, RenderOrchestrator* renderOrchestrator, StaticMeshHandle static_mesh_handle) {
		auto key = renderOrchestrator->GetBufferWriteKey(renderSystem, meshes[static_mesh_handle].NodeHandle, staticMeshInstanceDataStruct);
		auto pos = renderGroup->GetMeshTransform(static_mesh_handle);

		//info.MaterialSystem->UpdateIteratorMember(bufferIterator, staticMeshStruct, renderGroup->GetMeshIndex(e));
		renderOrchestrator->Write(renderSystem, key, matrixUniformBufferMemberHandle, pos);

		//if (rayTracing) { renderSystem->SetMeshMatrix(meshes[static_mesh_handle].RenderMesh, GTSL::Matrix3x4(pos)); }
		//TODO: MESHES ARE ONE THING, ACCELERATION STRUCTURE INSTANCES ARE OTHER
	}
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
		//renderSystem->SignalMeshDataUpdate(square, 4, 4 * 2, 6, 2, GTSL::StaticVector<GAL::ShaderDataType, 4>{ GAL::ShaderDataType::FLOAT2 });
		////
		//auto* meshPointer = renderSystem->GetMeshPointer(square);
		//GTSL::MemCopy(4 * 2 * 4, SQUARE_VERTICES, meshPointer);
		//meshPointer += 4 * 2 * 4;
		//GTSL::MemCopy(6 * 2, SQUARE_INDICES, meshPointer);
		//renderSystem->SignalMeshDataUpdate(square);
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
	
	RenderSystem::MeshHandle GetSquareMesh() const { return square; }
	MaterialInstanceHandle GetUIMaterial() const { return uiMaterial; }

private:
	RenderSystem::MeshHandle square;

	RenderOrchestrator::MemberHandle matrixUniformBufferMemberHandle, colorHandle;
	RenderOrchestrator::MemberHandle uiDataStruct;

	uint8 comps = 2;
	MaterialInstanceHandle uiMaterial;
};
