#pragma once

#include "ByteEngine/Game/System.hpp"

#include <GTSL/Bitfield.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/PagedVector.h>
#include <GTSL/SparseVector.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Tree.hpp>
#include <GTSL/Vector.hpp>

#include "ByteEngine/Render/Culling.h"
#include "ByteEngine/Render/RenderSystem.h"
#include "ByteEngine/Render/RenderTypes.h"
#include "ByteEngine/Render/UIManager.h"
#include "ByteEngine/Id.h"
#include "ByteEngine/Game/Tasks.h"
#include "ByteEngine/System/Resource/FontResourceManager.h"
#include "ByteEngine/System/Resource/ShaderResourceManager.h"
#include "ByteEngine/System/Resource/TextureResourceManager.h"

#include "ByteEngine/Graph.hpp"
#include "ByteEngine/Render/RenderSystem.h"
#include "GAL/RenderCore.h"
#include "GAL/RenderPass.h"

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

//Assists in determining a type's name when used in a shader, can assist validation
template<typename T>
struct TypeNamer {
	//if type is not known return empty
	static constexpr const char8_t* NAME = nullptr;
};

template<>
struct TypeNamer<GAL::DeviceAddress> {
	static constexpr const char8_t* NAME = u8"ptr_t";
};

template<>
struct TypeNamer<GTSL::float32> {
	static constexpr const char8_t* NAME = u8"float32";
};

template<>
struct TypeNamer<GTSL::Matrix3x4> {
	static constexpr const char8_t* NAME = u8"matrix3x4f";
};

inline void ToString(auto& string, const GTSL::Range<const GTSL::StaticString<32>*> range) {
	for (GTSL::uint32 i = 0; i < range.ElementCount(); ++i) {
		if (i) { string += u8", "; }
		string += range[i];
	}
}

/**
 * \brief Renders a frame according to a specfied model/pipeline.
 * E.J: Forward Rendering, Deferred Rendering, Ray Tracing, etc.
 */
class RenderPipeline : public BE::System {
public:
	RenderPipeline(const InitializeInfo& initialize_info, const char8_t* name) : System(initialize_info, name) {}
};

class RenderOrchestrator : public BE::System {
public:
	MAKE_HANDLE(GTSL::uint32, ElementData);

	enum class PassTypes : GTSL::uint8 {
		RASTER, COMPUTE, RAY_TRACING
	};

	enum class NodeType : GTSL::uint8 {
		DISPATCH, RAY_TRACE, MATERIAL, MESHES, RENDER_PASS, LAYER
	};

	struct Member {
		Member() = default;
		Member(const GTSL::StringView type, const GTSL::StringView name) : Type(type), Name(name) {}

		GTSL::StringView Type, Name;
	};

	struct MemberHandle {
		MemberHandle() = default;
		MemberHandle(const ElementDataHandle han) : Handle(han) {}

		ElementDataHandle Handle; GTSL::uint32 Index = 0;
	};

	struct NodeHandle {
		NodeHandle() = default;
		NodeHandle(const GTSL::uint32 val) : value(val) {}

		GTSL::uint32 operator()() const { return value; }

		operator bool() const { return value; }

		bool operator==(const NodeHandle& other) const {
			return value == other.value;
		}
	private:
		GTSL::uint32 value = 0;
	};

protected:
	MAKE_HANDLE(GTSL::uint64, Resource);

	struct AttachmentData {
		GTSL::StaticString<64> Name, Attachment;
		GAL::TextureLayout Layout;
		GAL::PipelineStage ConsumingStages;
		GAL::AccessType Access;
		GAL::Operations LoadOperation;
	};

	struct APIRenderPassData {
		GAL::RenderPass renderPass;
		GTSL::uint8 APISubPass = 0, SubPassCount = 0;
	};

public:
	struct MemberInfo : Member {
		MemberInfo() = default;
		MemberInfo(MemberHandle* memberHandle, GTSL::StringView type, GTSL::StringView name) : Member(type, name), Handle(memberHandle) {}
		MemberInfo(MemberHandle* memberHandle, GTSL::Range<MemberInfo*> memberInfos, GTSL::StringView type, GTSL::StringView name, const GTSL::uint32 alignment = 0) : Member(type, name), Handle(memberHandle), MemberInfos(memberInfos), alignment(alignment) {}

		MemberHandle* Handle = nullptr;
		GTSL::Range<MemberInfo*> MemberInfos;
		GTSL::uint16 alignment = 1;
	};

	explicit RenderOrchestrator(const InitializeInfo& initializeInfo);

	MAKE_HANDLE(GTSL::uint32, Set);

	struct SubSetDescription {
		SetHandle setHandle; GTSL::uint32 Subset;
		GAL::BindingType Type;
	};

	MAKE_HANDLE(SubSetDescription, SubSet);
	MAKE_HANDLE(GTSL::uint64, SetLayout);
	MAKE_HANDLE(GTSL::uint32, DataKey);

	// ------------ Data Keys ------------

	DataKeyHandle MakeDataKey() {
		auto pos = dataKeysMap.GetLength();
		dataKeysMap.EmplaceBack(dataKeys.Emplace(), 0u);
		return DataKeyHandle(pos);
	}

	[[nodiscard]] DataKeyHandle MakeDataKey(RenderSystem* renderSystem, const GTSL::StringView scope, const GTSL::StringView type, DataKeyHandle data_key_handle = DataKeyHandle(), GAL::BufferUse buffer_uses = GAL::BufferUse()) {
		RenderSystem::BufferHandle b[2]{};

		GTSL::StaticString<128> string(u8"Buffer: "); string << scope << u8"." << type;
		const auto handle = addMember(scope, type, string);

		const auto size = GetSize(handle.Get());

		b[0] = renderSystem->CreateBuffer(size, buffer_uses, true, b[0]); // Create host local, mappable buffer
		b[1] = renderSystem->CreateBuffer(size, buffer_uses, false, b[1]); // Create device local buffer to copy content into

		if(!data_key_handle) {
			data_key_handle = MakeDataKey();			
		}

		auto& dataKey = getDataKey(data_key_handle);

		dataKey.Buffer[0] = b[0];
		dataKey.Buffer[1] = b[1];
		dataKey.Handle = handle.Get();
		
		return data_key_handle;
	}

	void UpdateDataKey(const DataKeyHandle data_key_handle) {
		auto& dataKey = getDataKey(data_key_handle);

		for (auto& e : dataKey.Nodes) {
			SetNodeState(e, static_cast<bool>(dataKey.Buffer[0]) && static_cast<bool>(dataKey.Buffer[1]));
			renderingTree.UpdateNodeKey(e(), dataKeysMap[data_key_handle()].First);
			setRenderTreeAsDirty(e);
		}
	}

	void CopyDataKey(const DataKeyHandle from, const DataKeyHandle to, GTSL::uint32 offset) {
		if(from == to) { BE_LOG_WARNING(u8"Trying to transfer from same data key."); return; }

		{ // Scope variables since some will be invalidated after deletion
			auto& sourceDataKey = getDataKey(from);
			auto& destinationDataKey = getDataKey(to);

			if(sourceDataKey.Buffer[0]) {
				BE_LOG_WARNING(u8"Trying to delete data key handle: ", from(), u8", which contains initialized members.");
				return;
			}

			destinationDataKey.Nodes.PushBack(sourceDataKey.Nodes); // Transfer associated nodes

			sourceDataKey.Offset = offset;
		}

		dataKeys.Pop(dataKeysMap[from()].First); // Remove data key entry
		dataKeysMap[from()].First = dataKeysMap[to()].First; // Update entry pointer
		dataKeysMap[from()].Second = offset;
		UpdateDataKey(from);
	}

	// ------------ Data Keys ------------

	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo, RenderSystem* renderSystem);

	//HACKS, REMOVE
	NodeHandle GetGlobalDataLayer() const { return globalData; }
	//HACKS, REMOVE

	[[nodiscard]] RenderModelHandle CreateShaderGroup(GTSL::StringView shader_group_instance_name);

	void AddAttachment(GTSL::StringView attachment_name, GTSL::uint8 bitDepth, GTSL::uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type, bool is_multiframe);

	NodeHandle AddVertexBufferBind(RenderSystem* render_system, NodeHandle parent_node_handle, RenderSystem::BufferHandle buffer_handle, GTSL::Range<const GTSL::Range<const GAL::ShaderDataType*>*> meshVertexLayout) {
		auto nodeHandle = addInternalNode<VertexBufferBindData>(0, parent_node_handle);

		if(!nodeHandle) { return nodeHandle.Get(); }

		auto& node = getPrivateNode<VertexBufferBindData>(nodeHandle.Get());
		node.Handle = buffer_handle; node.VertexCount = 0; node.VertexSize = 0;

		{
			GTSL::uint32 offset = 0;

			for (auto& i : meshVertexLayout) {
				node.VertexSize += GAL::GraphicsPipeline::GetVertexSize(i);
			}

			auto bufferSize = render_system->GetBufferRange(buffer_handle).Bytes();

			for (auto& i : meshVertexLayout) {
				node.Offsets.EmplaceBack(offset);
				offset += GAL::GraphicsPipeline::GetVertexSize(i) * (bufferSize / node.VertexSize);
			}
		}


		return nodeHandle.Get();
	}

	void AddVertices(const NodeHandle node_handle, GTSL::uint32 count) {
		auto nodeType = renderingTree.GetNodeType(node_handle());

		setRenderTreeAsDirty(node_handle);

		if(nodeType == RTT::GetTypeIndex<VertexBufferBindData>()) {
			auto& node = getPrivateNode<VertexBufferBindData>(node_handle);
			node.VertexCount += count;
			return;
		}

		if(nodeType == RTT::GetTypeIndex<DrawData>()) {
			auto& node = getPrivateNode<DrawData>(node_handle);
			node.VertexCount += count;
			return;
		}		
	}

	NodeHandle AddIndexBufferBind(NodeHandle parent_node_handle, RenderSystem::BufferHandle buffer_handle) {
		auto nodeHandle = addInternalNode<IndexBufferBindData>(0, parent_node_handle);
		if(!nodeHandle) { return nodeHandle.Get(); }
		auto& node = getPrivateNode<IndexBufferBindData>(nodeHandle.Get());
		node.BufferHandle = buffer_handle; node.IndexCount = 0; node.IndexType = GAL::IndexType::GTSL::uint16;
		return nodeHandle.Get();
	}

	void AddIndices(const NodeHandle node_handle, GTSL::uint32 count) {
		auto& node = getPrivateNode<IndexBufferBindData>(node_handle);
		node.IndexCount += count;
		setRenderTreeAsDirty(node_handle);
	}

	void SetBaseInstanceIndex(NodeHandle node_handle, GTSL::uint32 base_instance_handle) {
		getPrivateNode<MeshData>(node_handle).InstanceIndex = base_instance_handle;
		setRenderTreeAsDirty(node_handle);
	}
	
	GTSL::uint32 GetInstanceIndex(const NodeHandle handle, const GTSL::uint32 instance_handle) {
		const auto& node = getPrivateNode<DataNode>(handle);
		return node.Instances[instance_handle];
	}

	template<typename T>
	GTSL::uint32 GetInstanceIndex(const NodeHandle handle, const T& instance_handle) {
		const auto& node = getPrivateNode<DataNode>(handle);
		return node.Instances[instance_handle()];
	}

	template<typename T>
	void AddInstance(NodeHandle data_node_handle, NodeHandle mesh_node_handle, T handle) {
		auto typeIndex = renderingTree.GetNodeType(mesh_node_handle());
		auto& dataNode = getPrivateNode<DataNode>(data_node_handle);

		dataNode.Instances.Emplace(handle(), dataNode.Instance);

		if(typeIndex == RTT::GetTypeIndex<MeshData>()) {
			auto& meshNode = getPrivateNode<MeshData>(mesh_node_handle);
			meshNode.InstanceIndex = !meshNode.InstanceCount ? dataNode.Instance : meshNode.InstanceIndex;
			meshNode.InstanceCount++;
			SetNodeState(mesh_node_handle, meshNode.InstanceCount);
		} else {
			auto& meshNode = getPrivateNode<DrawData>(mesh_node_handle);
			meshNode.InstanceCount++;
			SetNodeState(mesh_node_handle, meshNode.InstanceCount);
		}

		++dataNode.Instance;
	}

	struct BufferWriteKey {
		GTSL::uint32 Offset = 0;
		RenderSystem* render_system = nullptr; RenderOrchestrator* render_orchestrator = nullptr;
		RenderSystem::BufferHandle buffer_handle;
		GTSL::StaticString<256> Path{ u8"global" };
		ElementDataHandle ElementHandle;

		BufferWriteKey() {

		}

		BufferWriteKey(const BufferWriteKey&) = default;
		BufferWriteKey(GTSL::uint32 newOffset, GTSL::StringView path, const ElementDataHandle element_data_handle, const BufferWriteKey& other) : BufferWriteKey(other) { Offset = newOffset; Path = path; ElementHandle = element_data_handle; }

		BufferWriteKey operator[](const GTSL::uint32 index) {
			auto& e = render_orchestrator->getElement(ElementHandle);

			BE_ASSERT(render_orchestrator->getElement(ElementHandle).Type == ElementData::ElementType::MEMBER, u8"Type is not what it should be.");
			if(e.Mem.Multiplier == 1) {
				render_orchestrator->GetLogger()->PrintObjectLog(render_orchestrator, BE::Logger::VerbosityLevel::FATAL, u8"Tried to access ", Path, u8" as array but it isn't.");
				return BufferWriteKey{ 0xFFFFFFFF, Path, ElementHandle, *this };
			}

			if(index >= e.Mem.Multiplier) {
				render_orchestrator->GetLogger()->PrintObjectLog(render_orchestrator, BE::Logger::VerbosityLevel::FATAL, u8"Tried to access index ", index, u8" of ", Path, u8" but array size is ", e.Mem.Multiplier);
				return BufferWriteKey{ 0xFFFFFFFF, Path, ElementHandle, *this };
			}

			return BufferWriteKey{ Offset + render_orchestrator->GetSize(ElementHandle, true) * index, Path + u8"." + e.Name, ElementHandle, *this };
		}

		BufferWriteKey operator[](const GTSL::StringView path) {
			auto newPath = Path; newPath << u8"." << path;
			if(auto r = render_orchestrator->GetRelativeOffset(ElementHandle, path)) {
				return BufferWriteKey{ Offset + r.Get().Second, newPath, r.Get().First, *this };
			} else {
				render_orchestrator->GetLogger()->PrintObjectLog(render_orchestrator, BE::Logger::VerbosityLevel::FATAL, u8"Tried to access ", Path, u8" while writing, which doesn't exist.");
				return BufferWriteKey{ 0xFFFFFFFF, Path, ElementHandle, *this };
			}
		}

		BufferWriteKey operator()(const ElementDataHandle element_data_handle, const GTSL::uint32 offset) const {
			auto newPath = Path;
			return BufferWriteKey{ offset, newPath, element_data_handle, *this };
		}

		template<typename T>
		const BufferWriteKey& operator=(const T& obj) const {
			if (Offset == ~0u or !validateType<T>()) { return *this; }
			*reinterpret_cast<T*>(render_system->GetBufferPointer(buffer_handle) + Offset) = obj;
			return *this;
		}

		const BufferWriteKey& operator=(const BufferWriteKey& other) const {
			if (Offset == ~0u or GTSL::StringView(render_orchestrator->getElement(ElementHandle).DataType) != GTSL::StringView(render_orchestrator->getElement(other.ElementHandle).DataType)) { return *this; }
			auto& element = render_orchestrator->getElement(ElementHandle);
			GTSL::MemCopy(render_orchestrator->GetSize(element.Mem.TypeHandle), render_system->GetBufferPointer(other.buffer_handle), render_system->GetBufferPointer(buffer_handle) + Offset);
			return *this;
		}

		const BufferWriteKey& operator=(const RenderSystem::AccelerationStructureHandle acceleration_structure_handle) const {
			if (Offset == ~0u or !validateType<RenderSystem::AccelerationStructureHandle>()) { return *this; }
			*reinterpret_cast<GAL::DeviceAddress*>(render_system->GetBufferPointer(buffer_handle) + Offset) = render_system->GetTopLevelAccelerationStructureAddress(acceleration_structure_handle);
			return *this;
		}
		
		const BufferWriteKey& operator=(const RenderSystem::BufferHandle obj) const {
			if (Offset == ~0u or !validateType<RenderSystem::BufferHandle>()) { return *this; }
			*reinterpret_cast<GAL::DeviceAddress*>(render_system->GetBufferPointer(buffer_handle) + Offset) = render_system->GetBufferAddress(obj);
			return *this;
		}

		const BufferWriteKey& operator=(const DataKeyHandle obj) const {
			return (*this).operator=(render_orchestrator->dataKeys[obj()].Buffer[1]); // When copying copy destination buffer address
		}

		template<typename T>
		bool validateType() const {
			auto name = TypeNamer<T>::NAME;

			if(name) {
				if(render_orchestrator->getElement(render_orchestrator->getElement(ElementHandle).Mem.TypeHandle).Name == name) {
					return true;
				}

				render_orchestrator->GetLogger()->PrintObjectLog(render_orchestrator, BE::Logger::VerbosityLevel::FATAL, u8"Tried to access ", Path, u8" while writing, but types don't match.");
				return false;
			}

			return true;
		}
	};

	// ------------ Update Keys ------------

	MAKE_HANDLE(GTSL::uint32, UpdateKey)

	UpdateKeyHandle CreateUpdateKey() {
		auto index = updateKeys.GetLength();
		auto& updateKey = updateKeys.EmplaceBack();
		return UpdateKeyHandle(index);
	}

	UpdateKeyHandle GetShaderGroupIndexUpdateKey(RenderModelHandle shader_group_handle) {
		return shaderGroupInstances[shader_group_handle()].UpdateKey;
	}

	template<typename T>
	void WriteUpdateKey(RenderSystem* render_system, const UpdateKeyHandle update_key_handle, T val) {
		auto& updateKey = updateKeys[update_key_handle()];

		for(auto& e : updateKey.BWKs) {
			auto bwk = GetBufferWriteKey(render_system, e.DKH);
			bwk(e.EDH, e.Offset) = val;
		}

		updateKey.Value = val;
	}

	void SubscribeToUpdate(const UpdateKeyHandle update_key_handle, const BufferWriteKey buffer_write_key, const DataKeyHandle data_key_handle) {
		auto& updateKey = updateKeys[update_key_handle()];
		updateKey.BWKs.EmplaceBack(GTSL::MoveRef(data_key_handle), GTSL::MoveRef(buffer_write_key.ElementHandle), GTSL::MoveRef(buffer_write_key.Offset));
		buffer_write_key = updateKey.Value;
	}

	// ------------ Update Keys ------------

	GTSL::Delegate<void(RenderOrchestrator*, RenderSystem*)> shaderGroupNotify;
	DataKeyHandle globalDataDataKey, cameraDataKeyHandle;

	void AddNotifyShaderGroupCreated(GTSL::Delegate<void(RenderOrchestrator*, RenderSystem*)> notify_delegate) {
		shaderGroupNotify = notify_delegate;
	}

	struct ND
	{
		GTSL::StringView Name;
		DataKeyHandle DKH;
	};

	struct PassData {
		struct AttachmentReference {
			GTSL::StaticString<64> Name, Attachment;
			GAL::AccessType Access;
		};
		GTSL::StaticVector<AttachmentReference, 8> Attachments;

		PassTypes type;
	};
	NodeHandle AddRenderPassNode(NodeHandle parent_node_handle, GTSL::StringView instance_name, GTSL::StringView render_pass_name, RenderSystem*
	                             renderSystem, PassData pass_data, const GTSL::Range<const ND*> innner = {});

	void OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize);

	/**
	 * \brief Enables or disables the rendering of a render pass
	 * \param renderPassName Name of the render Pass to toggle
	 * \param enable Whether to enable(true) or disable(false) the render pass
	 */
	void ToggleRenderPass(NodeHandle renderPassName, bool enable);

	MAKE_HANDLE(GTSL::uint8, IndexStream) MAKE_HANDLE(GTSL::uint8, DataStream)

	void OnRenderEnable(TaskInfo taskInfo, bool oldFocus);
	void OnRenderDisable(TaskInfo taskInfo, bool oldFocus);

	MemberHandle CreateScope(const GTSL::StringView scope, const GTSL::StringView name) {
		return tryAddElement(scope, name, ElementData::ElementType::SCOPE).Get();
	}

	MemberHandle RegisterType(GTSL::StringView parents, GTSL::StringView structName, const GTSL::Range<const StructElement*> members) {
		GTSL::StaticVector<MemberInfo, 16> mem;

		for(auto& e : members) {
			mem.EmplaceBack(nullptr, e.Type, e.Name);
		}

		return RegisterType(parents, structName, mem);
	}

	MemberHandle RegisterType(GTSL::StringView parents, GTSL::StringView structName, const GTSL::Range<MemberInfo*> members) {
		auto parseMembers = [&](auto&& self, GTSL::StringView par, GTSL::StringView typeName, GTSL::StringView name, GTSL::Range<MemberInfo*> levelMembers, GTSL::uint16 level) -> ElementDataHandle {
			auto currentScope = GTSL::StaticString<128>(par) << u8"." << typeName;

			auto dataTypeEmplace = tryAddElement(par, typeName, ElementData::ElementType::TYPE);

			if(dataTypeEmplace.State() == 1) { //when element already exists clear data to redeclare element
				auto& e = getElement(dataTypeEmplace.Get());
				e.TyEl.Size = 0;
			}

			GTSL::uint32 offset = 0;

			for (GTSL::uint8 m = 0; m < levelMembers.ElementCount(); ++m) {
				auto& member = levelMembers[m];

				ElementDataHandle handle;

				if (member.MemberInfos.ElementCount()) {
					handle = self(self, currentScope, member.Type, member.Name, levelMembers[m].MemberInfos, level + 1);
					getElement(handle).TyEl.Alignment = 64;
				}

				handle = addMember(currentScope, member.Type, member.Name).Get();

				if (handle) {
					offset = GTSL::Math::RoundUpByPowerOf2(offset, static_cast<GTSL::uint32>(member.alignment));

					if (member.Handle) {
						*member.Handle = MemberHandle{ tryGetDataTypeHandle(currentScope, member.Type).Get() };
					}

					offset += GetSize(handle) * 1;
				}
			}

			return dataTypeEmplace.Get();
		};

		auto handle = parseMembers(parseMembers, parents, structName, u8"root", members, 0);
		return MemberHandle{ handle };
	}

	NodeHandle AddMaterial(NodeHandle parent_node_handle, RenderModelHandle materialHandle) {
		auto& shaderGroupInstance = shaderGroupInstances[materialHandle()];

		if(shaderGroupInstance.Name == u8"BlurH" || shaderGroupInstance.Name == u8"BlurV" || shaderGroupInstance.Name == u8"Barrel" || shaderGroupInstance.Name == u8"Floor") {
			auto materialDataNode = AddDataNode(parent_node_handle, shaderGroupInstance.Name, shaderGroupInstance.DataKey);
			auto pipelineBindNode = addPipelineBindNode(materialDataNode, materialHandle);
			auto& materialNode = getNode(pipelineBindNode);
			setNodeName(pipelineBindNode, shaderGroupInstance.Name);
			return pipelineBindNode;
		} else {
			auto pipelineBindNode = addPipelineBindNode(parent_node_handle, materialHandle);
			auto& materialNode = getNode(pipelineBindNode);
			setNodeName(pipelineBindNode, shaderGroupInstance.Name);
			return pipelineBindNode;
		}
	}

	NodeHandle AddMesh(const NodeHandle parentNodeHandle, GTSL::uint32 meshId, GTSL::uint32 indexCount, GTSL::uint32 indexOffset, GTSL::uint32 vertexOffset) {
		auto nodeHandle = addInternalNode<MeshData>(meshId, parentNodeHandle);
		if(!nodeHandle) { return nodeHandle.Get(); }
		
		getNode(nodeHandle.Get()).Name = GTSL::ShortString<32>(u8"Render Mesh");
		auto& node = getPrivateNode<MeshData>(nodeHandle.Get());
		node.IndexCount = indexCount;
		node.IndexOffset = indexOffset; node.VertexOffset = vertexOffset;
		return nodeHandle.Get();
	}
	
	void addPendingWrite(RenderSystem* render_system, RenderSystem::BufferHandle source_buffer_handle, RenderSystem::BufferHandle destination_buffer_handle) {
		auto key = GTSL::uint64(source_buffer_handle()) << 32;

		auto write = pendingWrites.TryEmplace(key);

		write.Get().FrameCountdown[render_system->GetCurrentFrame()] = true;
		write.Get().Buffer[0] = source_buffer_handle;
		write.Get().Buffer[1] = destination_buffer_handle;
	}

	NodeHandle AddDataNode(NodeHandle left_node_handle, NodeHandle parent, const DataKeyHandle data_key_handle) {
		auto nodeHandle = addInternalNode<DataNode>(data_key_handle(), left_node_handle, parent);
		if(!nodeHandle) { return nodeHandle.Get(); }

		auto& dataNode = getPrivateNode<DataNode>(nodeHandle.Get());

		auto& dataKey = dataKeys[data_key_handle()];
		dataKey.Nodes.EmplaceBack(nodeHandle.Get());
		UpdateDataKey(data_key_handle);
		setNodeName(nodeHandle.Get(), getElement(dataKey.Handle).Name);
		dataNode.DataKey = data_key_handle;
		dataNode.UseCounter = false;
		return nodeHandle.Get();
	}

	[[nodiscard]] NodeHandle AddDataNode(const NodeHandle parent_node_handle, const GTSL::StringView node_name, const DataKeyHandle data_key_handle, bool use_counter = false) {
		auto nodeHandle = addInternalNode<DataNode>(data_key_handle(), parent_node_handle);
		if(!nodeHandle) { return nodeHandle.Get(); }

		auto& dataNode = getPrivateNode<DataNode>(nodeHandle.Get());

		auto& dataKey = getDataKey(data_key_handle);
		dataKey.Nodes.EmplaceBack(nodeHandle.Get());
		UpdateDataKey(data_key_handle);
		setNodeName(nodeHandle.Get(), node_name);
		dataNode.DataKey = data_key_handle;
		dataNode.UseCounter = use_counter;
		return nodeHandle.Get();
	}

	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const DataKeyHandle data_key_handle) {
		const auto& dataKey = getDataKey(data_key_handle);
		BufferWriteKey buffer_write_key;
		buffer_write_key.render_system = render_system;
		buffer_write_key.render_orchestrator = this;
		buffer_write_key.buffer_handle = dataKey.Buffer[0];
		buffer_write_key.ElementHandle = dataKey.Handle;
		addPendingWrite(render_system, dataKey.Buffer[0], dataKey.Buffer[1]);
		return buffer_write_key;
	}

	void WriteBinding(RenderSystem* render_system, SubSetHandle subSetHandle, GTSL::uint32 bindingIndex, AccelerationStructure accelerationStructure) {
		for (GTSL::uint8 f = 0; f < render_system->GetPipelinedFrames(); ++f) {
			descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
		}
	}

	void WriteBinding(SubSetHandle subSetHandle, GTSL::uint32 bindingIndex, AccelerationStructure accelerationStructure, GTSL::uint8 f) {
		descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
	}

	void PushConstant(const RenderSystem* renderSystem, CommandList commandBuffer, SetLayoutHandle layout, GTSL::uint32 offset, GTSL::Range<const byte*> range) const {
		const auto& set = setLayoutDatas[layout()];
		commandBuffer.UpdatePushConstant(renderSystem->GetRenderDevice(), set.pipelineLayout, offset, range, set.Stage);
	}

	void BindSet(RenderSystem* renderSystem, CommandList commandBuffer, SetHandle setHandle, GAL::ShaderStage shaderStage) {
		auto& set = sets[setHandle()];
		commandBuffer.BindBindingsSets(renderSystem->GetRenderDevice(), shaderStage, GTSL::Range<BindingsSet*>(1, &set.bindingsSet[renderSystem->GetCurrentFrame()]), set.pipelineLayout, set.Level);
	}

	void WriteBinding(const RenderSystem* renderSystem, SubSetHandle setHandle, RenderSystem::TextureHandle textureHandle, GTSL::uint32 bindingIndex, GTSL::uint8 frameIndex) {
		GAL::TextureLayout layout; GAL::BindingType bindingType;

		if (setHandle().Type == GAL::BindingType::STORAGE_IMAGE) {
			layout = GAL::TextureLayout::GENERAL;
			bindingType = GAL::BindingType::STORAGE_IMAGE;
		}
		else {
			layout = GAL::TextureLayout::SHADER_READ;
			bindingType = GAL::BindingType::SAMPLED_IMAGE;
		}

		BindingsPool::TextureBindingUpdateInfo info;
		info.TextureView = *renderSystem->GetTextureView(textureHandle);
		info.Layout = layout;
		info.Format;

		descriptorsUpdates[frameIndex].AddTextureUpdate(setHandle, bindingIndex, info);
	}

	enum class SubSetTypes : GTSL::uint8 {
		BUFFER, READ_TEXTURES, WRITE_TEXTURES, RENDER_ATTACHMENT, ACCELERATION_STRUCTURE, SAMPLER
	};

	struct SubSetDescriptor {
		SubSetTypes type; GTSL::uint32 BindingsCount;
		SubSetHandle* Handle;
		GTSL::Range<const TextureSampler*> Sampler;
	};
	SetLayoutHandle AddSetLayout(RenderSystem* renderSystem, SetLayoutHandle parentName, const GTSL::Range<SubSetDescriptor*> subsets) {
		GTSL::uint64 hash = quickhash64(GTSL::Range(subsets.Bytes(), reinterpret_cast<const byte*>(subsets.begin())));

		SetLayoutHandle parentHandle;
		GTSL::uint32 level;

		if (parentName) {
			auto& parentSetLayout = setLayoutDatas[parentName()];
			parentHandle = parentName; level = parentSetLayout.Level + 1;
		} else {
			parentHandle = SetLayoutHandle(); level = 0;
		}

		auto& setLayoutData = setLayoutDatas.Emplace(hash);

		setLayoutData.Parent = parentHandle;
		setLayoutData.Level = level;

		GTSL::StaticVector<BindingsSetLayout, 16> bindingsSetLayouts;

		// Traverse tree to find parent's pipeline layouts
		{
			auto lastSet = parentHandle;

			for (GTSL::uint8 i = 0; i < level; ++i) { bindingsSetLayouts.EmplaceBack(); }

			for (GTSL::uint8 i = 0, l = level - 1; i < level; ++i, --l) {
				bindingsSetLayouts[l] = setLayoutDatas[lastSet()].bindingsSetLayout;
				lastSet = setLayoutDatas[lastSet()].Parent;
			}
		}

		setLayoutData.Stage = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::COMPUTE;

		GTSL::StaticVector<BindingsSetLayout::BindingDescriptor, 10> subSetDescriptors;

		for (const auto& e : subsets) {
			BindingsSetLayout::BindingDescriptor binding_descriptor;

			if (e.BindingsCount != 1) { binding_descriptor.Flags = GAL::BindingFlags::PARTIALLY_BOUND; }
			binding_descriptor.BindingsCount = e.BindingsCount;

			switch (e.type) {
			case SubSetTypes::BUFFER: binding_descriptor.Type = GAL::BindingType::STORAGE_BUFFER; break;
			case SubSetTypes::READ_TEXTURES: binding_descriptor.Type = GAL::BindingType::SAMPLED_IMAGE; break;
			case SubSetTypes::WRITE_TEXTURES: binding_descriptor.Type = GAL::BindingType::STORAGE_IMAGE; break;
			case SubSetTypes::RENDER_ATTACHMENT: binding_descriptor.Type = GAL::BindingType::INPUT_ATTACHMENT; break;
			case SubSetTypes::SAMPLER: {
				binding_descriptor.Type = GAL::BindingType::SAMPLER;
				binding_descriptor.Samplers = e.Sampler;
				binding_descriptor.BindingsCount = e.Sampler.ElementCount();
				break;
			}
			case SubSetTypes::ACCELERATION_STRUCTURE:
				binding_descriptor.Type = GAL::BindingType::ACCELERATION_STRUCTURE;
				binding_descriptor.Stage = GAL::ShaderStages::RAY_GEN;
				break;
			}

			binding_descriptor.Stage = setLayoutData.Stage;

			subSetDescriptors.EmplaceBack(binding_descriptor);
		}

		setLayoutData.bindingsSetLayout.Initialize(renderSystem->GetRenderDevice(), subSetDescriptors);
		bindingsSetLayouts.EmplaceBack(setLayoutData.bindingsSetLayout);

		GAL::PushConstant pushConstant;
		pushConstant.Stage = setLayoutData.Stage;
		pushConstant.NumberOf4ByteSlots = 32;
		setLayoutData.pipelineLayout.Initialize(renderSystem->GetRenderDevice(), &pushConstant, bindingsSetLayouts);

		return SetLayoutHandle(hash);
	}

	SetHandle AddSet(RenderSystem* renderSystem, GTSL::StringView setName, SetLayoutHandle setLayoutHandle, const GTSL::Range<SubSetDescriptor*> setInfo) {
		GTSL::StaticVector<BindingsSetLayout::BindingDescriptor, 16> bindingDescriptors;

		for (auto& ss : setInfo) {
			GAL::ShaderStage enabledShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::COMPUTE;

			switch (ss.type) {
			case SubSetTypes::BUFFER:
				bindingDescriptors.EmplaceBack(GAL::BindingType::STORAGE_BUFFER, enabledShaderStages, ss.BindingsCount, GAL::BindingFlags::PARTIALLY_BOUND);
				break;
			case SubSetTypes::READ_TEXTURES:
				bindingDescriptors.EmplaceBack(GAL::BindingType::SAMPLED_IMAGE, enabledShaderStages, ss.BindingsCount, GAL::BindingFlags::PARTIALLY_BOUND);
				break;
			case SubSetTypes::WRITE_TEXTURES:
				bindingDescriptors.EmplaceBack(GAL::BindingType::STORAGE_IMAGE, enabledShaderStages, ss.BindingsCount, GAL::BindingFlags::PARTIALLY_BOUND);
				break;
			case SubSetTypes::RENDER_ATTACHMENT:
				bindingDescriptors.EmplaceBack(GAL::BindingType::INPUT_ATTACHMENT, enabledShaderStages, ss.BindingsCount, GAL::BindingFlags::PARTIALLY_BOUND);
				break;
			case SubSetTypes::ACCELERATION_STRUCTURE:
				bindingDescriptors.EmplaceBack(GAL::BindingType::ACCELERATION_STRUCTURE, enabledShaderStages, ss.BindingsCount, GAL::BindingFlag());
				break;
			case SubSetTypes::SAMPLER:
				bindingDescriptors.EmplaceBack(GAL::BindingType::SAMPLER, enabledShaderStages, ss.BindingsCount, GAL::BindingFlag());
				break;
			default:;
			}
		}

		auto setHandle = makeSetEx(renderSystem, Id(setName), setLayoutHandle, bindingDescriptors);

		auto& set = sets[setHandle()];
		GTSL::uint32 i = 0;
		for (auto& ss : setInfo) {
			*ss.Handle = SubSetHandle({ setHandle, i, bindingDescriptors[i].Type });
			++i;
		}

		return setHandle;
	}

	struct BindingsSetData {
		BindingsSetLayout Layout;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		GTSL::uint32 DataSize = 0;
	};

	// Evaluates a node's state variables and sets it's enable state accordingly
	// Used to enable a node only if dependencies have been fulfilled
	void revalNode(const NodeHandle node_handle) {
		auto& node = getNode(node_handle);
		const bool fulfilled = node.References >= node.L;
		const bool nodeState = node.Enabled && fulfilled;
		if(nodeState != renderingTree.GetNodeState(node_handle())) {
			renderingTree.ToggleBranch(node_handle(), nodeState);
			setRenderTreeAsDirty(node_handle);
		}

		if(BE::Application::Get()->GetConfig()[u8"RenderOrchestrator"][u8"debugResourceFulfillment"].GetBool()) {
			if(nodeState) {
				BE_LOG_MESSAGE(u8"Node: ", node.Name, u8", was enabled.");
			} else {
				BE_LOG_MESSAGE(u8"Node: ", node.Name, u8", was disabled.");
			}
		}
	}

	void AddNodeDependency(const NodeHandle node_handle) {
		auto& node = getNode(node_handle);
		++node.L;
		revalNode(node_handle);
	}

	void FulfillNodeDependency(const NodeHandle node_handle) {
		auto& node = getNode(node_handle);
		++node.References;
		revalNode(node_handle);
	}

	void SetNodeState(const NodeHandle node_handle, const bool state) {
		auto& node = getNode(node_handle);
		node.Enabled = state;
		revalNode(node_handle);
	}

	void PrintMember(const DataKeyHandle data_key_handle, RenderSystem* render_system) const {
		byte* beginPointer;

		GTSL::SemiString<BE::TAR, 4096> string(u8"\n", GetTransientAllocator()); //start struct on new line, looks better when printed

		const auto& dataKey = getDataKey(data_key_handle);
		const GTSL::uint32 startOffset = dataKey.Offset;

		auto walkTree = [&](const ElementDataHandle member_handle, GTSL::uint32 level, GTSL::uint32 offset, auto&& self) -> GTSL::uint32 {
			auto& e = elements[member_handle()];
			auto& dt = getElement(e.Mem.TypeHandle);

			GTSL::StaticString<128> dtt;

			for (GTSL::uint32 t = 0; t < e.Mem.Multiplier && t < 4; ++t) { // Clamp printed array elements to 16
				string += u8"\n";

				for (GTSL::uint32 i = 0; i < level; ++i) { string += U'	'; } //insert tab for every space deep we are to show struct depth

				dtt = e.DataType;

				string += u8"offset: "; ToString(string, offset - startOffset); string += u8", "; string += dt.DataType; string += u8" ";
				if(e.Mem.Multiplier > 1) {
					string += '['; GTSL::ToString(string, t); string += u8"] ";
					dtt = dt.Name;
				}
				string += e.Name; string += u8": ";


				if (FindLast(dt.DataType, U'*')) {
					GTSL::ToString(string, reinterpret_cast<GTSL::uint64*>(beginPointer + offset)[0]);
				}
				else {
					switch (GTSL::Hash(dtt)) {
					case GTSL::Hash(u8"ptr_t"): {
						GTSL::ToString(string, reinterpret_cast<GTSL::uint64*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"GTSL::uint32"): {
						GTSL::ToString(string, reinterpret_cast<GTSL::uint32*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"GTSL::uint64"): {
						GTSL::ToString(string, reinterpret_cast<GTSL::uint64*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"float32"): {
						GTSL::ToString(string, reinterpret_cast<float32*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"TextureReference"): {
						auto textureHandle = reinterpret_cast<GTSL::uint32*>(beginPointer + offset)[0];

						GTSL::ToString(string, textureHandle); string += u8", ";
						break;
					}
					case GTSL::Hash(u8"ImageReference"): {
						GTSL::ToString(string, reinterpret_cast<GTSL::uint32*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"vec2f"): {
						auto pointer = reinterpret_cast<GTSL::Vector2*>(beginPointer + offset)[0];
						GTSL::ToString(string, pointer.X()); string += u8", ";
						GTSL::ToString(string, pointer.Y());
						break;
					}
					case GTSL::Hash(u8"vec2u"): {
						auto pointer = reinterpret_cast<GTSL::uint32*>(beginPointer + offset);
						GTSL::ToString(string, pointer[0]); string += u8", ";
						GTSL::ToString(string, pointer[1]);
						break;
					}
					case GTSL::Hash(u8"vec3f"): {
						auto pointer = reinterpret_cast<GTSL::Vector3*>(beginPointer + offset)[0];
						GTSL::ToString(string, pointer.X()); string += u8", ";
						GTSL::ToString(string, pointer.Y()); string += u8", ";
						GTSL::ToString(string, pointer.Z());
						break;
					}
					case GTSL::Hash(u8"vec4f"): {
						auto pointer = reinterpret_cast<GTSL::Vector4*>(beginPointer + offset)[0];
						GTSL::ToString(string, pointer.X());
						GTSL::ToString(string, pointer.Y());
						GTSL::ToString(string, pointer.Z());
						GTSL::ToString(string, pointer.W());
						break;
					}
					case GTSL::Hash(u8"matrix3x4f"): {
						auto matrixPointer = reinterpret_cast<GTSL::Matrix3x4*>(beginPointer + offset)[0];

						for (GTSL::uint8 r = 0; r < 3; ++r) {
							for (GTSL::uint32 i = 0; i < level && r; ++i) { string += U'	'; } //insert tab for every space deep we are to show struct depth

							for (GTSL::uint8 c = 0; c < 4; ++c) {
								GTSL::ToString(string, matrixPointer[r][c]); string += u8" ";
							}

							string += U'\n';
						}

						break;
					}
					case GTSL::Hash(u8"matrix4f"): {
						auto matrixPointer = reinterpret_cast<GTSL::Matrix4*>(beginPointer + offset)[0];

						for (GTSL::uint8 r = 0; r < 4; ++r) {
							for (GTSL::uint32 i = 0; i < level && r; ++i) { string += U'	'; } //insert tab for every space deep we are to show struct depth

							for (GTSL::uint8 c = 0; c < 4; ++c) {
								GTSL::ToString(string, matrixPointer[r][c]); string += u8" ";
							}

							string += U'\n';
						}

						break;
					}
					case GTSL::Hash(u8"ShaderHandle"): {

						for (GTSL::uint32 i = 0; i < 4; ++i) {
							GTSL::uint64 val = reinterpret_cast<GTSL::uint64*>(beginPointer + offset)[i];
							if (i) { string << u8"-"; } ToString(string, val);
						}

						GTSL::uint64 shaderHandleHash = quickhash64({ 32, reinterpret_cast<byte*>(beginPointer + offset) });

						if (auto r = shaderHandlesDebugMap.TryGet(shaderHandleHash)) {
							string << u8", handle for shader: ";
							GTSL::ToString(string, r.Get());
						}
						else {
							string << u8", shader handle not found.";
						}

						break;
					}
					}
				}

				GTSL::uint32 size = 0;

				for (auto& e : dt.children) {
					if (getElement(e.Handle).Type == ElementData::ElementType::MEMBER) {
						size = GTSL::Math::RoundUpByPowerOf2(size, getElement(getElement(ElementDataHandle(e.Handle)).Mem.TypeHandle).TyEl.Alignment);
						size += self(e.Handle, level + 1, offset + size, self);
					}
				}

				offset += dt.TyEl.Size;

				BE_ASSERT(dt.Type == ElementData::ElementType::TYPE, u8"Type is not what it should be.");
			}

			return dt.TyEl.Size * e.Mem.Multiplier; //todo: align
		};
		
		beginPointer = render_system->GetBufferPointer(dataKey.Buffer[0]);
		string += u8"\nAddress: "; GTSL::ToString(string, static_cast<GTSL::uint64>(render_system->GetBufferAddress(dataKey.Buffer[0])));
		string += u8"\n";
		walkTree(ElementDataHandle(dataKey.Handle), 0, startOffset, walkTree);

		BE_LOG_MESSAGE(string);
	}

	NodeHandle AddSquare(const NodeHandle parent_node_handle) {
		auto nodeHandle = addInternalNode<DrawData>(0, parent_node_handle);
		if(!nodeHandle) { return nodeHandle.Get(); }
		setNodeName(nodeHandle.Get(), u8"Square");
		getPrivateNode<DrawData>(nodeHandle.Get()).VertexCount = 6;
		SetNodeState(nodeHandle.Get(), false);
		return nodeHandle.Get();
	}

	NodeHandle AddRayTraceNode(const NodeHandle parent_node_handle, const RenderModelHandle material_instance_handle) {
		auto handle = addInternalNode<RayTraceData>(222, parent_node_handle);
		if(!handle) { return handle.Get(); }
		getPrivateNode<RayTraceData>(handle.Get()).ShaderGroupIndex = material_instance_handle();
		return handle.Get();
	}

private:
	inline static const auto RENDER_TASK_NAME{ u8"RenderOrchestrator::Render" };
	inline static const auto SETUP_TASK_NAME{ u8"RenderOrchestrator::Setup" };
	inline static const auto CLASS_NAME{ u8"RenderOrchestrator" };

	inline static constexpr GTSL::uint32 RENDER_DATA_BUFFER_SIZE = 262144;
	inline static constexpr GTSL::uint32 RENDER_DATA_BUFFER_SLACK_SIZE = 4096;
	inline static constexpr GTSL::uint32 RENDER_DATA_BUFFER_PAGE_SIZE = RENDER_DATA_BUFFER_SIZE + RENDER_DATA_BUFFER_SLACK_SIZE;

	void onRenderEnable(ApplicationManager* gameInstance, const GTSL::Range<const TaskDependency*> dependencies);
	void onRenderDisable(ApplicationManager* gameInstance);

	bool renderingEnabled = false;

	GTSL::uint32 renderDataOffset = 0;
	SetLayoutHandle globalSetLayout;
	SetHandle globalBindingsSet;
	NodeHandle rayTraceNode;

	MemberHandle cameraMatricesHandle;
	MemberHandle globalDataHandle;
	SubSetHandle textureSubsetsHandle;
	SubSetHandle imagesSubsetHandle;
	SubSetHandle samplersSubsetHandle;

	RenderSystem::CommandListHandle graphicsCommandLists[MAX_CONCURRENT_FRAMES];
	RenderSystem::CommandListHandle buildCommandList[MAX_CONCURRENT_FRAMES], transferCommandList[MAX_CONCURRENT_FRAMES];

	RenderSystem::WorkloadHandle graphicsWorkloadHandle[MAX_CONCURRENT_FRAMES], buildAccelerationStructuresWorkloadHandle[MAX_CONCURRENT_FRAMES];

	GTSL::HashMap<Id, GTSL::uint32, BE::PAR> rayTracingSets;

	GTSL::StaticVector<GTSL::StaticVector<GTSL::StaticVector<GAL::ShaderDataType, 8>, 8>, 16> vertexLayouts;

	GTSL::HashMap<GTSL::uint64, GTSL::StaticString<128>, BE::PAR> shaderHandlesDebugMap;

	struct RenderState {
		GAL::ShaderStage ShaderStages;
		GTSL::uint8 streamsCount = 0, buffersCount = 0;

		GTSL::uint32 BoundPipelineIndex, BoundShaderGroupIndex;

		DataKeyHandle dataKeys[128 / 8];

		DataStreamHandle AddDataStream(const DataKeyHandle data_key_handle) {
			dataKeys[buffersCount] = data_key_handle;
			++buffersCount;
			return DataStreamHandle(streamsCount++);
		}

		void PopData() {
			--streamsCount; --buffersCount;
		}
	};

	struct ShaderData {
		GAL::VulkanShader Shader;
		GAL::ShaderType Type;
		GTSL::StaticString<64> Name;
	};
	GTSL::HashMap<GTSL::uint64, ShaderData, BE::PAR> shaders;

	struct MeshData {
		GTSL::uint32 InstanceCount = 0;
		GTSL::uint32_t IndexCount, IndexOffset, VertexOffset, InstanceIndex;
	};

	struct DispatchData {
		GTSL::Extent3D DispatchSize;
	};

	struct PipelineBindData {
		RenderModelHandle Handle;
	};

	struct RayTraceData {
		GTSL::uint32 ShaderGroupIndex = 0xFFFFFFFF;
	};

	struct RenderPassData {
		PassTypes Type;
		GTSL::StaticVector<AttachmentData, 16> Attachments;
		GAL::PipelineStage PipelineStages;
		MemberHandle RenderTargetReferences;
		ResourceHandle resourceHandle;
		DataKeyHandle DataKey;
	};

	struct DataNode {
		DataKeyHandle DataKey;
		bool UseCounter;
		GTSL::uint32 Instance = 0;
		GTSL::HashMap<GTSL::uint32, GTSL::uint32, GTSL::DefaultAllocatorReference> Instances;
	};

	struct PublicNode {
		GTSL::ShortString<32> Name;
		NodeType Type; GTSL::uint8 Level = 0;
		GTSL::uint32 InstanceCount = 0;
		GTSL::uint32 References = 0, L = 0;
		bool Enabled = true;
	};

	//Node's names are nnot provided inn the CreateNode functions since we don't want to generate debug names in release builds, and the compiler won't eliminate the useless string generation code
	//if it were provided in the less easy to see through CreateNode functions
	void setNodeName(const NodeHandle internal_node_handle, const GTSL::StringView name) {
		if constexpr (BE_DEBUG) { getNode(internal_node_handle).Name = name; }
	}

	[[nodiscard]] NodeHandle addNode(const GTSL::StringView nodeName, const NodeHandle parent, const NodeType layerType) {
		auto l = addNode(nodeName, parent, layerType);
		auto& t = getNode(l);
		t.Name = nodeName;
		return l;
	}

	PublicNode& getNode(const NodeHandle nodeHandle) {
		return renderingTree.GetAlpha(nodeHandle());
	}

	template<class T>
	T& getPrivateNode(const NodeHandle internal_node_handle) {
		return renderingTree.GetClass<T>(internal_node_handle());
	}

	NodeHandle globalData;

	void transitionImages(CommandList commandBuffer, RenderSystem* renderSystem, const RenderPassData* internal_layer);

	struct ShaderLoadInfo {
		ShaderLoadInfo() = default;
		ShaderLoadInfo(const BE::PAR& allocator) noexcept : Buffer(allocator), MaterialIndex(0) {}
		ShaderLoadInfo(ShaderLoadInfo&& other) noexcept : Buffer(MoveRef(other.Buffer)), MaterialIndex(other.MaterialIndex), handle(other.handle) {}
		GTSL::Buffer<BE::PAR> Buffer; GTSL::uint32 MaterialIndex;
		NodeHandle handle;
	};

	GTSL::uint64 resourceCounter = 0;

	ResourceHandle makeResource(const GTSL::StringView resource_name) {
		auto& resource = resources.Emplace(++resourceCounter);
		resource.Name = resource_name;
		return ResourceHandle(resourceCounter);
	}

	void BindResourceToNode(NodeHandle node_handle, ResourceHandle resource_handle) {
		if (!resources.Find(resource_handle())) { BE_LOG_ERROR(u8"Invalid resource handle: ", resource_handle()); return; }

		auto& resource = resources[resource_handle()];

		resource.NodeHandles.EmplaceBack(node_handle);

		SetNodeState(node_handle, resource.isValid());
	}

	void addDependencyOnResource(const ResourceHandle resourceHandle) {
		if (!resources.Find(resourceHandle())) { BE_LOG_ERROR(u8"Invalid resource handle: ", resourceHandle()); return; }
		++resources[resourceHandle()].Target;
	}

	void addDependencyOnResource(const ResourceHandle waiterHandle, const ResourceHandle providerHandle) {
		if (!resources.Find(waiterHandle())) { BE_LOG_ERROR(u8"Invalid resource handle: ", waiterHandle()); return; }

		auto& provider = resources[providerHandle()]; auto& waiter = resources[waiterHandle()];

		provider.Children.EmplaceBack(waiterHandle);

		++waiter.Target;

		bool enableValue = waiter.isValid();

		for (auto e : waiter.NodeHandles) {
			SetNodeState(e, enableValue);
		}
	}

	void signalDependencyToResource(ResourceHandle resource_handle) {
		if (resources.Find(resource_handle())) {
			tryEnableResource(resource_handle);
		}
		else {
			BE_LOG_WARNING(u8"Tried to enable resource: ", resource_handle(), u8" which is not available.");
		}
	}

	void tryEnableResource(ResourceHandle resource_handle) {
		auto& resource = resources[resource_handle()];
		++resource.Count;
		if (resource.isValid()) {
			for (auto e : resource.Children) {
				tryEnableResource(e);
			}

			for (const auto& e : resource.NodeHandles) {
				SetNodeState(e, true);
			}
		}
	}

	struct ResourceData {
		GTSL::ShortString<32> Name;
		GTSL::StaticVector<NodeHandle, 8> NodeHandles;
		GTSL::uint32 Count = 0, Target = 0;
		GTSL::StaticVector<ResourceHandle, 8> Children;

		bool isValid() const { return Count >= Target; }
	};
	GTSL::HashMap<GTSL::uint64, ResourceData, BE::PAR> resources;

	// ------------ Data Keys ------------

	struct DataKeyData {
		GTSL::uint32 Offset = 0;
		RenderSystem::BufferHandle Buffer[2];
		GTSL::StaticVector<NodeHandle, 8> Nodes;
		ElementDataHandle Handle;
	};
	GTSL::FixedVector<DataKeyData, BE::PAR> dataKeys;
	GTSL::Vector<GTSL::Pair<GTSL::uint32, GTSL::uint32>, BE::PAR> dataKeysMap;

	DataKeyData& getDataKey(const DataKeyHandle data_key_handle) {
		return dataKeys[dataKeysMap[data_key_handle()].First];
	}

	const DataKeyData& getDataKey(const DataKeyHandle data_key_handle) const {
		return dataKeys[dataKeysMap[data_key_handle()].First];
	}

	// ------------ Data Keys ------------

	void onShaderInfosLoaded(TaskInfo taskInfo, ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo shaderInfos, ShaderLoadInfo shaderLoadInfo);

	void onShadersLoaded(TaskInfo taskInfo, ShaderResourceManager*, RenderSystem*, ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo);

	struct DrawData {
		GTSL::uint32 VertexCount = 0, InstanceCount = 0;
	};

	struct VertexBufferBindData {
		GTSL::uint32 VertexCount = 0, VertexSize = 0;
		RenderSystem::BufferHandle Handle;
		GTSL::StaticVector<GTSL::uint32, 8> Offsets;
	};

	struct IndexBufferBindData {
		GTSL::uint32 IndexCount = 0;
		GAL::IndexType IndexType;		
		RenderSystem::BufferHandle BufferHandle;
	};

	struct IndirectComputeDispatchData {
		
	};

	GTSL::MultiTree<BE::PAR, PublicNode, PipelineBindData, DataNode, RayTraceData, DispatchData, MeshData, RenderPassData, DrawData, VertexBufferBindData, IndexBufferBindData, IndirectComputeDispatchData> renderingTree;
	using RTT = decltype(renderingTree);
	
	bool isCommandBufferUpdated[MAX_CONCURRENT_FRAMES]{ false };
	bool isRenderTreeDirty = false;

	void setRenderTreeAsDirty(const NodeHandle dirty_node_handle) { isRenderTreeDirty = true; }

	template<typename T>
	GTSL::Result<NodeHandle> addInternalNode(const GTSL::uint64 key, NodeHandle publicParentHandle) {
		auto betaNodeHandle = renderingTree.Emplace<T>(key, 0xFFFFFFFF, publicParentHandle());
		setRenderTreeAsDirty(publicParentHandle);
		return { NodeHandle(betaNodeHandle.Get()), betaNodeHandle.State() };
	}

	template<typename T>
	GTSL::Result<NodeHandle> addInternalNode(const GTSL::uint64 key, NodeHandle leftNodeHandle, NodeHandle publicParentHandle) {
		auto betaNodeHandle = renderingTree.Emplace<T>(key, leftNodeHandle(), publicParentHandle());
		setRenderTreeAsDirty(publicParentHandle);
		return { NodeHandle(betaNodeHandle.Get()), betaNodeHandle.State() };
	}

	NodeHandle addPipelineBindNode(const NodeHandle parent_node_handle, const RenderModelHandle material_instance_handle) {
		auto handle = addInternalNode<PipelineBindData>(555, parent_node_handle);

		if(!handle) { return handle.Get(); }

		getPrivateNode<PipelineBindData>(handle.Get()).Handle = material_instance_handle;
		BindResourceToNode(handle.Get(), shaderGroupInstances[material_instance_handle()].Resource);
		return handle.Get();
	}

	static auto parseScopeString(const GTSL::StringView parents) {
		GTSL::StaticVector<GTSL::StaticString<64>, 8> strings;

		{
			GTSL::uint32 i = 0;

			while (i < parents.GetCodepoints()) {
				auto& string = strings.EmplaceBack();

				while (parents[i] != U'.' and i < parents.GetCodepoints()) {
					string += parents[i];
					++i;
				}

				++i;
			}
		}

		return strings;
	}

	GTSL::HashMap<GTSL::StringView, GTSL::uint32, BE::PAR> renderPassesMap;
	GTSL::StaticVector<NodeHandle, 32> renderPasses;

	struct Pipeline {
		Pipeline(const BE::PAR& allocator) {}

		GPUPipeline pipeline;
		//ResourceHandle ResourceHandle;
		DataKeyHandle ShaderBindingTableBuffer;

		GTSL::StaticVector<GTSL::uint64, 16> Shaders;

		struct RayTracingData {
			struct ShaderGroupData {
				MemberHandle TableHandle;

				struct InstanceData {
					MemberHandle ShaderHandle;
					GTSL::StaticVector<MemberHandle, 8> Elements;
				};

				GTSL::uint32 ShaderCount = 0;

				GTSL::StaticVector<InstanceData, 8> Instances;
			} ShaderGroups[4];

			GTSL::uint32 PipelineIndex;
		} RayTracingData;

		GTSL::StaticString<64> ExecutionString;
	};
	GTSL::FixedVector<Pipeline, BE::PAR> pipelines;

	struct ShaderGroupData {
		GTSL::StaticString<64> Name;
		DataKeyHandle Buffer;
		GTSL::StaticMap<Id, MemberHandle, 16> ParametersHandles;
		GTSL::StaticVector<ShaderResourceManager::Parameter, 16> Parameters;
		bool Loaded = false;
		GTSL::uint32 RasterPipelineIndex = 0xFFFFFFFF, ComputePipelineIndex = 0xFFFFFFFF, RTPipelineIndex = 0xFFFFFFFF;
		ResourceHandle Resource;
		GTSL::StaticVector<StructElement, 8> PushConstantLayout;
	};
	GTSL::FixedVector<ShaderGroupData, BE::PAR> shaderGroups;

	struct ShaderGroupInstanceData {
		GTSL::StaticString<64> Name;
		ResourceHandle Resource;
		GTSL::uint32 ShaderGroupIndex = 0;
		DataKeyHandle DataKey;
		UpdateKeyHandle UpdateKey;
	};
	GTSL::StaticVector<ShaderGroupInstanceData, 32> shaderGroupInstances;

	GTSL::HashMap<GTSL::StringView, GTSL::uint32, BE::PAR> shaderGroupsByName, shaderGroupInstanceByName;

	GTSL::uint32 textureIndex = 0, imageIndex = 0;

	void printNode(const GTSL::uint32 nodeHandle, GTSL::uint32 level, bool d, bool e) {
		if (!d) { return; }

		GTSL::StaticString<256> message;

		message += u8"Node: "; GTSL::ToString(message, nodeHandle); message += u8", Depth: ", GTSL::ToString(message, level); message += u8", Type: ";

		switch (renderingTree.GetNodeType(nodeHandle)) {
		case decltype(renderingTree)::GetTypeIndex<DataNode>(): { message += u8"DataNode"; break; }
		case decltype(renderingTree)::GetTypeIndex<PipelineBindData>(): { message += u8"PipelineBind"; break; }
		case decltype(renderingTree)::GetTypeIndex<MeshData>(): { message += u8"MeshDraw"; break; }
		case decltype(renderingTree)::GetTypeIndex<VertexBufferBindData>(): { message += u8"VertexBufferBind"; break; }
		case decltype(renderingTree)::GetTypeIndex<IndexBufferBindData>(): { message += u8"IndexBufferBind"; break; }
		case decltype(renderingTree)::GetTypeIndex<RenderPassData>(): { message += u8"RenderPass"; break; }
		case decltype(renderingTree)::GetTypeIndex<DrawData>(): { message += u8"Draw"; break; }
		case decltype(renderingTree)::GetTypeIndex<IndirectComputeDispatchData>(): { message += u8"Dispatch"; break; }
		case decltype(renderingTree)::GetTypeIndex<RayTraceData>(): { message += u8"Raytrace"; break; }
		case decltype(renderingTree)::GetTypeIndex<DispatchData>(): { message += u8"Compute Dispatch"; break; }
		default: { message += u8"null"; break; }
		}

		message += u8", Name: "; message += getNode(nodeHandle).Name;

		if(e) {
			BE_LOG_MESSAGE(message)
		} else {
			message += u8", Unfulfilled dependencies: ";

			GTSL::StaticVector<GTSL::StaticString<32>, 16> deps;

			for(auto& e : resources) {
				if(e.NodeHandles.Find(NodeHandle(nodeHandle))) {
					if (!e.isValid()) { //todo: recurse
						deps.EmplaceBack(e.Name);
					}
				}
			}

			ToString(message, deps);

			BE_LOG_WARNING(message)
		}
	}

	struct CreateTextureInfo {
		GTSL::ShortString<64> TextureName;
		ApplicationManager* applicationManager = nullptr;
		RenderSystem* renderSystem = nullptr;
		TextureResourceManager* textureResourceManager = nullptr;
	};
	GTSL::uint32 createTexture(const CreateTextureInfo& createTextureInfo);

	struct MaterialLoadInfo {
		MaterialLoadInfo(RenderSystem* renderSystem, GTSL::Buffer<BE::PAR>&& buffer, GTSL::uint32 index, GTSL::uint32 instanceIndex, TextureResourceManager* tRM) : renderSystem(renderSystem), Buffer(MoveRef(buffer)), Component(index), InstanceIndex(instanceIndex), textureResourceManager(tRM)
		{
		}

		RenderSystem* renderSystem = nullptr;
		GTSL::Buffer<BE::PAR> Buffer;
		GTSL::uint32 Component, InstanceIndex;
		TextureResourceManager* textureResourceManager;
	};

	struct TextureLoadInfo {
		TextureLoadInfo() = default;

		TextureLoadInfo(RenderAllocation renderAllocation) : allocation(renderAllocation)
		{}

		RenderAllocation allocation;
		RenderSystem::TextureHandle TextureHandle;
	};
	void onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem*, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);
	void onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem*, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);

	struct TextureData {
		ResourceHandle Resource;
		GTSL::uint32 Index = 0;
	};
	GTSL::HashMap<GTSL::StringView, TextureData, BE::PAR> textures;

	void addPendingResourceToTexture(GTSL::StringView texture, ResourceHandle resource) {
		addDependencyOnResource(resource, textures[texture].Resource);
	}

	struct Attachment {
		RenderSystem::TextureHandle TextureHandle[MAX_CONCURRENT_FRAMES];

		GTSL::StaticString<64> Name;
		GAL::TextureUse Uses; GAL::TextureLayout Layout[MAX_CONCURRENT_FRAMES];
		GAL::PipelineStage ConsumingStages; GAL::AccessType AccessType;
		GTSL::RGBA ClearColor; GAL::FormatDescriptor format;
		GTSL::uint32 ImageIndeces[MAX_CONCURRENT_FRAMES];
		//bool IsMultiFrame = false;
	};
	GTSL::HashMap<GTSL::StringView, Attachment, BE::PAR> attachments;

	void updateImage(GTSL::uint8 frameIndex, Attachment& attachment, GAL::TextureLayout textureLayout, GAL::PipelineStage stages, GAL::AccessType writeAccess) {
		attachment.Layout[frameIndex] = textureLayout; attachment.ConsumingStages = stages; attachment.AccessType = writeAccess;
	}

	TaskHandle<TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureInfoLoadHandle;
	TaskHandle<TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureLoadHandle;
	TaskHandle<ShaderResourceManager::ShaderGroupInfo, ShaderLoadInfo> onShaderInfosLoadHandle;
	TaskHandle<ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*>, ShaderLoadInfo> onShaderGroupLoadHandle;

	struct ElementData {
		ElementData(const BE::PAR& allocator) : children() {}

		enum class ElementType {
			NONE, SCOPE, TYPE, MEMBER
		} Type = ElementType::NONE;

		GTSL::StaticString<64> DataType, Name;

		struct Member {
			ElementDataHandle TypeHandle;
			GTSL::uint32 Alignment = 1;
			GTSL::uint32 Multiplier;
		} Mem;

		struct TypeElement {
			GTSL::uint32 Size = 0, Alignment = 1;
		} TyEl;

		struct Entry {
			GTSL::StaticString<64> Name;
			ElementDataHandle Handle;
		};
		GTSL::StaticVector<Entry, 64> children;
	};
	GTSL::Tree<ElementData, BE::PAR> elements;

	void addScope(const GTSL::StringView scope, const GTSL::StringView name) {
		tryAddElement(scope, name, ElementData::ElementType::SCOPE);
	}

	GTSL::Result<ElementDataHandle> addMember(const GTSL::StringView scope, const GTSL::StringView type, const GTSL::StringView name) {
		auto parents = parseScopeString(scope);

		ElementDataHandle parentHandle, typeHandle;

		auto typeString = GTSL::StaticString<128>(type);
		GTSL::uint32 multiplier = 1;

		if (auto pos = FindFirst(typeString, u8'[')) {
			GTSL::uint32 i = pos.Get();

			while(i < typeString.GetCodepoints()) {
				while(i < typeString.GetCodepoints() && typeString[i++] != u8'[') {}

				GTSL::uint32 start = i;

				while(i < typeString.GetCodepoints() && typeString[i++] != u8']') {}

				GTSL::uint32 end = i - 1;

				multiplier *= GTSL::ToNumber<GTSL::uint32>({ end - start, end - start, typeString.c_str() + start }).Get();
			}

			typeString.Drop(pos.Get());
		}

		if(auto r = tryGetDataTypeHandle(scope, typeString)) {
			typeHandle = r.Get();
		} else {
			BE_LOG_WARNING(u8"Failed to create member.");
			return { ElementDataHandle(), false };
		}

		{
			BE_ASSERT(getElement(typeHandle).Type == ElementData::ElementType::TYPE, u8"");

			auto elementResult = tryAddElement(scope, name, ElementData::ElementType::MEMBER);
			auto& element = getElement(elementResult.Get());
			element.Mem.TypeHandle = typeHandle;
			element.Mem.Alignment = getElement(typeHandle).TyEl.Alignment;
			element.Mem.Multiplier = multiplier;
			element.DataType = type;

			for (GTSL::uint32 i = 1, j = parents.GetLength() - 1; i < parents; ++i, --j) {
				auto& t = tryGetDataTypeHandle(scope, parents[j]).Get();
				auto& ttt = getElement(t);
				if (ttt.Type != ElementData::ElementType::TYPE) { break; }
				//BE_LOG_MESSAGE(u8"Pre size: ", ttt.TyEl.Size, u8", handle: ", t(), u8", name: ", ttt.Name);
				ttt.TyEl.Size = GTSL::Math::RoundUpByPowerOf2(ttt.TyEl.Size, getElement(typeHandle).TyEl.Alignment);
				ttt.TyEl.Size += getElement(typeHandle).TyEl.Size * multiplier;
				//BE_LOG_MESSAGE(u8"Post size: ", ttt.TyEl.Size, u8", handle: ", t(), u8", name: ", ttt.Name);
			}

			return { (ElementDataHandle&&)elementResult.Get(), true };
		}

	}

	//will return the handle to name element under parents scope
	GTSL::Result<ElementDataHandle> tryGetDataTypeHandle(GTSL::Range<const GTSL::StringView*> parents, GTSL::StringView name) {
		if (*(name.end() - 1) == U'*') {
			return tryGetDataTypeHandle(u8"global", u8"ptr_t");
		}

		ElementDataHandle handle{ 1 };

		for (auto& e : parents) {
			if (e == u8"global") {
				handle = ElementDataHandle(1);
			} else {
				if (auto r = GTSL::Find(elements[handle()].children, [&](const ElementData::Entry& entry) { return entry.Name == e; })) {
					handle = ElementDataHandle(r.Get()->Handle);
				} else {
					break;
				}
			}

			if (auto r = GTSL::Find(elements[handle()].children, [&](const ElementData::Entry& entry) { return name == entry.Name; })) {
				return { ElementDataHandle(r.Get()->Handle), true };
			}
		}

		return { ElementDataHandle(), false };
	}

	GTSL::Result<ElementDataHandle> tryGetDataTypeHandle(GTSL::StringView scope) {
		auto scopes = parseScopeString(scope);

		ElementDataHandle handle{ 1 };

		for (GTSL::uint32 i = 0; i < scopes.GetLength(); ++i) {
			if (scopes[i] == u8"global") {
				handle = ElementDataHandle(1);
			} else {
				if (auto r = GTSL::Find(elements[handle()].children, [&](const ElementData::Entry& entry) { return scopes[i] == entry.Name; })) {
					handle = r.Get()->Handle;
				} else {
					return { ElementDataHandle(), false };
				}
			}
		}

		return { ElementDataHandle(handle), true };
	}

	GTSL::Result<ElementDataHandle> tryGetDataTypeHandle(GTSL::StringView parents, GTSL::StringView name) {
		GTSL::StaticVector<GTSL::StringView, 8> pppp;

		auto t = parseScopeString(parents);

		for (auto& e : t) {
			pppp.EmplaceBack(e);
		}

		return tryGetDataTypeHandle(pppp, name);
	}

	GTSL::Result<ElementDataHandle> tryGetDataTypeHandle(ElementDataHandle parent, GTSL::StringView name) {
		if (*(name.end() - 1) == U'*') {
			return tryGetDataTypeHandle(u8"global", u8"ptr_t");
		}

		if (auto r = GTSL::Find(getElement(parent).children, [&](const ElementData::Entry& entry) { return name == entry.Name; })) {
			return { ElementDataHandle(r.Get()->Handle), true };
		}

		return { ElementDataHandle(), false };
	}

	//will declare data type name under parents
	//2 result if added
	//1 result if exists
	//0 result if failed
	GTSL::Result<ElementDataHandle, GTSL::uint8> tryAddElement(const GTSL::StringView parents, const GTSL::StringView name, ElementData::ElementType type) {
		auto parentList = parseScopeString(parents); //parse parent list and make array

		ElementDataHandle parentHandle;

		if(auto r = tryGetDataTypeHandle(parents)) {
			parentHandle = r.Get();
		} else {
			return { ElementDataHandle(), 0 };
		}

		auto entry = tryEmplaceChild(name, parentHandle);

		if (!entry) {
			return { ElementDataHandle(entry.Get()), 1 };
		}

		auto& child = elements[entry.Get()()];
		child.Name = name;
		child.Type = type;
		return { ElementDataHandle(entry.Get()), 2 };
	}

	ElementData& getElement(const ElementDataHandle element_data_handle) {
		return elements[element_data_handle()];
	}

	const ElementData& getElement(const ElementDataHandle element_data_handle) const {
		return elements[element_data_handle()];
	}

	GTSL::Result<ElementDataHandle> tryAddDataType(const GTSL::StringView parents, const GTSL::StringView name, GTSL::uint32 size) {
		if (auto r = tryAddElement(parents, name, ElementData::ElementType::TYPE); r.State()) {
			getElement(r.Get()).TyEl.Size = size;
			return { ElementDataHandle(r.Get()), (bool)r.State() };
		} else {
			getElement(r.Get()).TyEl.Size = size;
			return { ElementDataHandle(r.Get()), (bool)r.State() };
		}
	}

	GTSL::Result<ElementDataHandle> tryEmplaceChild(const GTSL::StringView name, ElementDataHandle parentHandle) {
		auto res = GTSL::Find(elements[parentHandle()].children, [&](const ElementData::Entry& entry) { return name == entry.Name; });

		if(!res) {
			auto newChildIndex = elements.Emplace(parentHandle(), GetPersistentAllocator());
			auto& newChild = elements[newChildIndex];
			newChild.Name = name;
			elements[parentHandle()].children.EmplaceBack(name, ElementDataHandle(newChildIndex));

			return { ElementDataHandle(newChildIndex), true};
		}

		return { ElementDataHandle(res.Get()->Handle), false};
	}

	GTSL::Result<GTSL::Pair<ElementDataHandle, GTSL::uint32>> GetRelativeOffset(const ElementDataHandle element_data_handle, const GTSL::StringView newScope) const {
		ElementDataHandle handle = element_data_handle;

		auto getOffset = [&](const GTSL::StringView scope) -> GTSL::Result<GTSL::Pair<ElementDataHandle, GTSL::uint32>> {
			GTSL::uint32 offset = 0;

			if (handle != ElementDataHandle(1)) { //if we are not in global scope
				if (getElement(handle).Type == ElementData::ElementType::MEMBER) {
					handle = getElement(handle).Mem.TypeHandle;
				}

				for (auto& k : elements[handle()].children) {
					auto& t = getElement(k.Handle);

					if(t.Type != ElementData::ElementType::MEMBER) { continue; }

					offset = GTSL::Math::RoundUpByPowerOf2(offset, getElement(t.Mem.TypeHandle).TyEl.Alignment);
					if (k.Name == newScope) { return { { k.Handle, static_cast<GTSL::uint32&&>(offset) }, true }; }
					offset += getElement(t.Mem.TypeHandle).TyEl.Size * t.Mem.Multiplier;
				}
			}			

			return { { ElementDataHandle(), 0 }, false };
		};

		return getOffset(newScope);
	}

	void updateDescriptors(TaskInfo taskInfo) {
		auto* renderSystem = taskInfo.AppManager->GetSystem<RenderSystem>(u8"RenderSystem");

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
						bindingsUpdateInfo.BindingsSet = &sets[set.First].bindingsSet[renderSystem->GetCurrentFrame()];
						bindingsUpdateInfo.SubsetIndex = b.First;

						for (auto& t : a) {
							bindingsUpdateInfo.BindingIndex = t.First;
							bindingsUpdateInfo.BindingUpdateInfos = t.GetElements();
							bindingsUpdateInfos.EmplaceBack(bindingsUpdateInfo);
						}
					}
				}

				sets[set.First].bindingsPool[renderSystem->GetCurrentFrame()].Update(renderSystem->GetRenderDevice(), bindingsUpdateInfos, GetTransientAllocator());
			}
		}

		descriptorsUpdate.Reset();
	}

	static constexpr GAL::BindingType BUFFER_BINDING_TYPE = GAL::BindingType::STORAGE_BUFFER;

	struct DescriptorsUpdate {
		DescriptorsUpdate(const BE::PAR& allocator) : sets(16, allocator) {
		}

		void AddBufferUpdate(SubSetHandle subSetHandle, GTSL::uint32 binding, BindingsPool::BufferBindingUpdateInfo update) {
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void AddTextureUpdate(SubSetHandle subSetHandle, GTSL::uint32 binding, BindingsPool::TextureBindingUpdateInfo update) {
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void AddAccelerationStructureUpdate(SubSetHandle subSetHandle, GTSL::uint32 binding, BindingsPool::AccelerationStructureBindingUpdateInfo update) {
			addUpdate(subSetHandle, binding, BindingsPool::BindingUpdateInfo(update));
		}

		void Reset() {
			sets.Clear();
		}

		GTSL::SparseVector<GTSL::SparseVector<GTSL::SparseVector<BindingsPool::BindingUpdateInfo, BE::PAR>, BE::PAR>, BE::PAR> sets;

	private:
		void addUpdate(SubSetHandle subSetHandle, GTSL::uint32 binding, BindingsPool::BindingUpdateInfo update) {
			if (sets.IsSlotOccupied(subSetHandle().setHandle())) {
				auto& set = sets[subSetHandle().setHandle()];

				if (set.IsSlotOccupied(subSetHandle().Subset)) {
					auto& subSet = set[subSetHandle().Subset];

					if (subSet.IsSlotOccupied(binding)) {
						subSet[binding] = update;
					}
					else { //there isn't binding
						subSet.EmplaceAt(binding, update);
					}
				}
				else {//there isn't sub set
					auto& subSet = set.EmplaceAt(subSetHandle().Subset, 32, sets.GetAllocator());
					//subSet.First = bindingType;
					subSet.EmplaceAt(binding, update);
				}
			}
			else { //there isn't set
				auto& set = sets.EmplaceAt(subSetHandle().setHandle(), 16, sets.GetAllocator());
				auto& subSet = set.EmplaceAt(subSetHandle().Subset, 32, sets.GetAllocator());
				subSet.EmplaceAt(binding, update);
			}
		}
	};

	GTSL::StaticVector<DescriptorsUpdate, MAX_CONCURRENT_FRAMES> descriptorsUpdates;

	GTSL::uint32 GetSize(const MemberHandle member_handle) {
		return GetSize(ElementDataHandle(member_handle.Handle));
	}

	GTSL::uint32 GetSize(const ElementDataHandle element_data_handle, bool getOnlyType = false) {
		auto& e = elements[element_data_handle()];

		switch (e.Type) {
		case ElementData::ElementType::NONE: break;
		case ElementData::ElementType::SCOPE: break;
		case ElementData::ElementType::TYPE: return e.TyEl.Size;
		case ElementData::ElementType::MEMBER: return getElement(e.Mem.TypeHandle).TyEl.Size * (getOnlyType ? 1 : e.Mem.Multiplier);
		}

		BE_ASSERT(false, u8"Should not reach here");

		return 0;
	}

	/**
	 * \brief Stores all data per binding set.
	 */
	struct SetData {
		Id Name;
		//SetHandle Parent;
		GTSL::uint32 Level = 0;
		PipelineLayout pipelineLayout;
		BindingsSetLayout bindingsSetLayout;
		BindingsPool bindingsPool[MAX_CONCURRENT_FRAMES];
		BindingsSet bindingsSet[MAX_CONCURRENT_FRAMES];

		/**
		 * \brief Stores all data per sub set, and manages managed buffers.
		 * Each struct instance is pointed to by one binding. But a big per sub set buffer is used to store all instances.
		 */
		struct SubSetData {
			GAL::BindingType Type;
			GTSL::uint32 AllocatedBindings = 0;
		};
		GTSL::StaticVector<SubSetData, 16> SubSets;
	};
	GTSL::FixedVector<SetData, BE::PAR> sets;
	GTSL::PagedVector<SetHandle, BE::PAR> queuedSetUpdates;

	GTSL::StaticVector<GAL::VulkanSampler, 16> samplers;

	struct SetLayoutData {
		GTSL::uint8 Level = 0;

		SetLayoutHandle Parent;
		BindingsSetLayout bindingsSetLayout;
		PipelineLayout pipelineLayout;
		GAL::ShaderStage Stage;
	};
	GTSL::HashMap<GTSL::uint64, SetLayoutData, BE::PAR> setLayoutDatas;

	SetHandle makeSetEx(RenderSystem* renderSystem, Id setName, SetLayoutHandle setLayoutHandle, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDescriptors) {
		auto setHandle = SetHandle(sets.Emplace());
		auto& set = sets[setHandle()];

		auto& setLayout = setLayoutDatas[setLayoutHandle()];

		set.Level = setLayout.Level;
		set.bindingsSetLayout = setLayout.bindingsSetLayout;
		set.pipelineLayout = setLayout.pipelineLayout;

		if (bindingDescriptors.ElementCount()) {
			GTSL::StaticVector<BindingsPool::BindingsPoolSize, 10> bindingsPoolSizes;

			for (auto& e : bindingDescriptors) {
				bindingsPoolSizes.EmplaceBack(BindingsPool::BindingsPoolSize{ e.Type, e.BindingsCount * renderSystem->GetPipelinedFrames() });
				set.SubSets.EmplaceBack(); auto& subSet = set.SubSets.back();
				subSet.Type = e.Type;
				subSet.AllocatedBindings = e.BindingsCount;
			}

			for (GTSL::uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
				set.bindingsPool[f].Initialize(renderSystem->GetRenderDevice(), bindingsPoolSizes, 1);
				set.bindingsSet[f].Initialize(renderSystem->GetRenderDevice(), set.bindingsPool[f], setLayout.bindingsSetLayout);
			}
		}

		return setHandle;
	}

	friend class WorldRendererPipeline;

	byte* buffer[MAX_CONCURRENT_FRAMES]; GTSL::uint32 offsets[MAX_CONCURRENT_FRAMES]{ 0 };

	struct PendingWriteData {
		RenderSystem::BufferHandle Buffer[2];
		bool FrameCountdown[MAX_CONCURRENT_FRAMES] = { false };
	};
	GTSL::HashMap<GTSL::uint64, PendingWriteData, BE::PAR> pendingWrites;

	GTSL::ShortString<16> tag;

	static constexpr bool INVERSE_Z = true;

	GTSL::Math::RandomSeed randomA, randomB;

	GTSL::uint32 bnoise[4] = { 0 };

	GTSL::uint32 frameIndex = 0;

	struct UpdateKeyData {
		struct ttt
		{
			DataKeyHandle DKH;
			ElementDataHandle EDH;
			GTSL::uint32 Offset;
		};
		GTSL::StaticVector<ttt, 8> BWKs;
		GTSL::uint32 Value;
	};
	GTSL::Vector<UpdateKeyData, BE::PAR> updateKeys;

#if BE_DEBUG
	GAL::PipelineStage pipelineStages;

	struct DebugView {
		GTSL::StaticString<64> name;
		WindowSystem::WindowHandle windowHandle;
		RenderSystem::RenderContextHandle renderContext;
		RenderSystem::WorkloadHandle workloadHandles[MAX_CONCURRENT_FRAMES];
		GTSL::Extent2D sizeHistory[MAX_CONCURRENT_FRAMES];
	};
	GTSL::StaticVector<DebugView, 8> views;
#endif

	void parseRenderPassJSON() {
		GTSL::JSON<BE::PAR> json(GetPersistentAllocator());

		GTSL::HashMap<GTSL::StringView, GTSL::StaticString<64>, BE::PAR> allAttachments(GetPersistentAllocator());
		GTSL::HashMap<GTSL::StringView, Graph<GTSL::uint32>, BE::PAR> renderPassNodes(GetPersistentAllocator());

		for(auto renderPass : json) {
			auto name = renderPass[u8"name"];

			auto& node = renderPassNodes.Emplace(name, 0u);

			for(auto attachment : renderPass[u8"attachments"]) {
				auto name = attachment[u8"name"];

				allAttachments.TryEmplace(name, name);

				auto use = attachment[u8"use"];

				if(GTSL::StringView(use) == u8"INPUT") {
					
				} else if (GTSL::StringView(use) == u8"OUTPUT") {
					
				} else {
					// TODO: error
				}
			}

			for(auto dependsOn : renderPass[u8"dependsOn"]) {
				renderPassNodes[dependsOn].Connect(node);
			}
		}

		GTSL::Vector<GTSL::StaticString<64>, BE::PAR> fullAttachments(GetPersistentAllocator()), transientAttachments(GetPersistentAllocator());
	}
};

inline GTSL::uint64 Hash(char8_t c) { return c; }

class UIRenderManager : public RenderManager {
public:
	DECLARE_BE_TASK(OnCreateUIElement, BE_RESOURCES(RenderOrchestrator, UIManager), UIManager::UIElementHandle, UIManager::PrimitiveData::PrimitiveType);

	DECLARE_BE_TASK(OnFontLoad, BE_RESOURCES(RenderSystem, RenderOrchestrator), FontResourceManager::FontData, GTSL::Buffer<BE::PAR>);

	UIRenderManager(const InitializeInfo& initializeInfo) : RenderManager(initializeInfo, u8"UIRenderManager"), instancesMap(32, GetPersistentAllocator()), charToGlyphMap(GetPersistentAllocator()), characters(GetPersistentAllocator()) {
		auto* renderSystem = initializeInfo.AppManager->GetSystem<RenderSystem>(u8"RenderSystem");
		auto* renderOrchestrator = initializeInfo.AppManager->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");

		auto tickTaskHandle = GetApplicationManager()->RegisterTask(this, u8"uiEveryFrame", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"), TypedDependency<UIManager>(u8"UIManager")), &UIRenderManager::everyFrame, u8"RenderSetup", u8"Render");
		GetApplicationManager()->EnqueueScheduledTask(tickTaskHandle);

		//TODO: check why setting an end stage stop the whole process
		OnCreateUIElementTaskHandle = GetApplicationManager()->RegisterTask(this, u8"OnCreateUIElement", DependencyBlock(TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"), TypedDependency<UIManager>(u8"UIManager")), &UIRenderManager::OnCreateUIElement);

		GetApplicationManager()->SubscribeToEvent(u8"UIManager", UIManager::GetOnCreateUIElementEventHandle(), OnCreateUIElementTaskHandle);
		GetApplicationManager()->AddTypeSetupDependency(this, GetApplicationManager()->GetSystem<UIManager>(u8"UIManager")->GetUIElementTypeIdentifier(), OnCreateUIElementTaskHandle);

		renderOrchestrator->CreateScope(u8"global", u8"UI");

		renderOrchestrator->RegisterType(u8"global.UI", u8"TextData", UI_TEXT_DATA);
		renderOrchestrator->RegisterType(u8"global.UI", u8"LinearSegment", UI_LINEAR_SEGMENT);
		renderOrchestrator->RegisterType(u8"global.UI", u8"QuadraticSegment", UI_QUADRATIC_SEGMENT);
		renderOrchestrator->RegisterType(u8"global.UI", u8"GlyphContourData", UI_GLYPH_CONTOUR_DATA);
		renderOrchestrator->RegisterType(u8"global.UI", u8"GlyphData", UI_GLYPH_DATA);
		renderOrchestrator->RegisterType(u8"global.UI", u8"FontData", UI_FONT_DATA);

		renderOrchestrator->RegisterType(u8"global.UI", u8"UIInstance", UI_INSTANCE_DATA);
		uiInstancesDataKey = renderOrchestrator->MakeDataKey(renderSystem, u8"global.UI", u8"UIInstance[16]");

		renderOrchestrator->RegisterType(u8"global.UI", u8"UIData", UI_DATA);
		uiDataDataKey = renderOrchestrator->MakeDataKey(renderSystem, u8"global.UI", u8"UIData");

		{
			RenderOrchestrator::PassData uiRenderPassData;
			uiRenderPassData.type = RenderOrchestrator::PassTypes::RASTER;
			uiRenderPassData.Attachments.EmplaceBack(GTSL::StringView(u8"UI"), GTSL::StringView(u8"UI"), GAL::AccessTypes::WRITE);
			auto renderPassNodeHandle = renderOrchestrator->AddRenderPassNode(renderOrchestrator->GetGlobalDataLayer(), u8"UI", u8"UIRenderPass", renderSystem, uiRenderPassData);

			auto uiDataNodeHandle = renderOrchestrator->AddDataNode(renderPassNodeHandle, u8"UIData", uiDataDataKey);
			uiInstancesDataNodeHandle = renderOrchestrator->AddDataNode(uiDataNodeHandle, u8"UIInstancesData", uiInstancesDataKey, true);

			uiMaterialNodeHandle = renderOrchestrator->AddMaterial(uiInstancesDataNodeHandle, renderOrchestrator->CreateShaderGroup(u8"UI"));
			textMaterialNodeHandle = renderOrchestrator->AddMaterial(uiInstancesDataNodeHandle, renderOrchestrator->CreateShaderGroup(u8"UIText"));
		}

		meshNodeHandle = renderOrchestrator->AddSquare(uiMaterialNodeHandle);
		textMeshNodeHandle = renderOrchestrator->AddSquare(textMaterialNodeHandle);

		// Load font data
		OnFontLoadTaskHandle = GetApplicationManager()->RegisterTask(this, u8"OnFontLoad", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &UIRenderManager::OnFontLoad);

		auto* fontResourceManager = GetApplicationManager()->GetSystem<FontResourceManager>(u8"FontResourceManager");
		fontResourceManager->LoadFont(u8"COOPBL", OnFontLoadTaskHandle);
		// Load font data
	}

	void OnCreateUIElement(const TaskInfo, RenderOrchestrator* render_orchestrator, UIManager* ui_manager, UIManager::UIElementHandle ui_element_handle, UIManager::PrimitiveData::PrimitiveType type) {
		switch (type) {
		break; case UIManager::PrimitiveData::PrimitiveType::NONE:
		break; case UIManager::PrimitiveData::PrimitiveType::CANVAS:
		break; case UIManager::PrimitiveData::PrimitiveType::ORGANIZER:
		break; case UIManager::PrimitiveData::PrimitiveType::SQUARE:
			render_orchestrator->AddInstance(uiInstancesDataNodeHandle, meshNodeHandle, ui_element_handle);
		break; case UIManager::PrimitiveData::PrimitiveType::TEXT: {
			auto string = ui_manager->GetString(ui_element_handle());

			for(GTSL::uint32 i = 0; i < string.GetCodepoints(); ++i) {
				render_orchestrator->AddInstance(uiInstancesDataNodeHandle, textMeshNodeHandle, ui_element_handle);
			}
		}
		break; case UIManager::PrimitiveData::PrimitiveType::CURVE:
		break;
		}

		instancesMap.Emplace(ui_element_handle(), 0);
	}

	//void ui() {
	//	UIManager* ui;
	//	UIManager::TextPrimitive textPrimitive{ GetPersistentAllocator() };
	//
	//	RenderOrchestrator::BufferWriteKey text;
	//	text[u8"fontIndex"] = fontOrderMap[Id(textPrimitive.Font)];
	//
	//	for (GTSL::uint32 i = 0; i < textPrimitive.Text; ++i) {
	//		text[u8"chars"][i] = fontCharMap[textPrimitive.Text[i]];
	//	}
	//
	//	for(const auto& e : ui->GetCanvases()) {
	//	}
	//
	//	//ui->GetText
	//}

	static GTSL::Matrix4 MakeOrthoMatrix(GTSL::Vector2 extent, const float32 nearPlane, const float32 farPlane) {
		float32 w = extent.X() / extent.Y();

		GTSL::Matrix4 matrix;
		matrix[0][0] = static_cast<float32>(2) / (extent.X() - -extent.X());
		matrix[1][1] = static_cast<float32>(2) / (extent.Y() - -extent.Y());
		matrix[2][2] = static_cast<float32>(1) / (farPlane - nearPlane);
		matrix[0][3] = -(w + -w) / (w - -w);
		matrix[1][3] = -(1.0f + -1.0f) / (1.0f - -1.0f);
		matrix[2][3] = -nearPlane / (farPlane - nearPlane);

		//matrix[0][3] = 0.0f;
		//matrix[1][3] = 0.0f;
		//matrix[2][3] = -nearPlane / (farPlane - nearPlane);

		return matrix;
	}

	void everyFrame(TaskInfo, RenderSystem* render_system, RenderOrchestrator* render_orchestrator,  UIManager* ui) {
		ui->ProcessUpdates();

		float32 r = 0;

		{
			// TODO: value can be outdated
			auto windowExtent = GTSL::Extent2D(1920, 1080);
			auto windowSize = GTSL::Vector2(static_cast<float32>(windowExtent.Width), static_cast<float32>(windowExtent.Height));
			auto windowNormalizedSize = GTSL::Vector2(float32(windowExtent.Width) / static_cast<float32>(windowExtent.Height), 1.0f);

			auto screenExtent = GTSL::Extent2D(1920, 1080);
			auto screenSize = GTSL::Vector2(screenExtent.Width, screenExtent.Height);
			auto screenNormalizedSize = GTSL::Vector2(screenSize.X() / screenSize.Y(), 1.0f);
			
			auto renderSize = screenNormalizedSize * (windowSize / screenSize);

			r = GTSL::Math::LengthSquared(windowSize) / GTSL::Math::LengthSquared(screenSize);

			auto bwk = render_orchestrator->GetBufferWriteKey(render_system, uiDataDataKey);

			GTSL::Matrix4 projectionMatrix;

			if constexpr (UIManager::WINDOW_SPACE) {
				projectionMatrix = MakeOrthoMatrix(windowNormalizedSize, 0.0f, 1.f);
			} else {
				projectionMatrix = MakeOrthoMatrix(renderSize, 0.0f, 1.f);
			}

			//auto projectionMatrix = GTSL::Matrix4();
			projectionMatrix[1][1] *= API == GAL::RenderAPI::VULKAN ? -1.0f : 1.0f;
			bwk[u8"projection"] = projectionMatrix;
		}

		auto root = ui->GetRoot();

		auto uiData = render_orchestrator->GetBufferWriteKey(render_system, uiDataDataKey);
		auto bwk = render_orchestrator->GetBufferWriteKey(render_system, uiInstancesDataKey);

		auto visitUIElement = [&](GTSL::Tree<UIManager::PrimitiveData, BE::PAR>::iterator iterator, GTSL::Matrix3x4 matrix, auto&& self) -> void {
			if(!instancesMap.Find(iterator.GetHandle())) { return; }

			const auto& primitive = static_cast<const UIManager::PrimitiveData&>(iterator);

			GTSL::Matrix3x4 primitiveMatrix;

			//if(!primitive.isDirty) { return; }

			switch (primitive.Type) {
			break; case UIManager::PrimitiveData::PrimitiveType::NONE:
			break; case UIManager::PrimitiveData::PrimitiveType::CANVAS:
			break; case UIManager::PrimitiveData::PrimitiveType::ORGANIZER:
			break; case UIManager::PrimitiveData::PrimitiveType::SQUARE: {
				GTSL::Math::Scale(primitiveMatrix, GTSL::Vector3(primitive.RenderSize, 0));
				GTSL::Math::Translate(primitiveMatrix, GTSL::Vector3(primitive.Position, 0));

				const auto i = render_orchestrator->GetInstanceIndex(uiInstancesDataNodeHandle, iterator.GetHandle());

				bwk[i][u8"transform"] = primitiveMatrix;
				bwk[i][u8"color"] = GTSL::Vector4(primitive.Color);
				bwk[i][u8"roundness"] = primitive.Rounding;
			}
			break; case UIManager::PrimitiveData::PrimitiveType::TEXT: {
				uiData[u8"textData"][0][u8"fontIndex"] = 0u;

				auto string = ui->GetString(iterator.GetHandle());

				float32 x = primitive.Position.X() + primitive.RenderSize.X() * -1.f;

				for(GTSL::uint32 i = 0; auto c : string) {
					if(!charToGlyphMap.Find(c)) { break; }
					auto glyphIndex = charToGlyphMap[c];
					auto& character = characters[glyphIndex];

					uiData[u8"textData"][0][u8"chars"][i] = glyphIndex;

					const auto index = render_orchestrator->GetInstanceIndex(uiInstancesDataNodeHandle, iterator.GetHandle());

					//GTSL::Math::Scale(primitiveMatrix, GTSL::Vector3(primitive.RenderSize * GTSL::Vector2(character.Bearing.X, character.Bearing.Y) * 0.01f, 0));
					GTSL::Math::Scale(primitiveMatrix, GTSL::Vector3(primitive.RenderSize, 0));
					GTSL::Math::Translate(primitiveMatrix, GTSL::Vector3(x, primitive.Position.Y(), 0));

					bwk[index][u8"transform"] = primitiveMatrix;
					bwk[index][u8"color"] = GTSL::Vector4(primitive.Color);
					bwk[index][u8"roundness"] = primitive.Rounding;
					bwk[index][u8"derivedTypeIndex"][0] = 0; // Text
					bwk[index][u8"derivedTypeIndex"][1] = glyphIndex; // Char

					// now advance cursors for next glyph (note that advance is number of 1/64 pixels)
					x += (characters[glyphIndex].Advance >> 6); // bitshift by 6 to get value in pixels (2^6 = 64)

					++i;
				}


				//float xpos = x + ch.Bearing.X;
				//float ypos = y - (ch.Size.Height - ch.Bearing.Y);
			}
			break; case UIManager::PrimitiveData::PrimitiveType::CURVE:
				break;
			}

			for (auto e : iterator) {
				self(e, primitiveMatrix, self);
			}
		};

		visitUIElement(root, GTSL::Matrix3x4(), visitUIElement);
	}

	void OnFontLoad(TaskInfo, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, FontResourceManager::FontData font_data, GTSL::Buffer<BE::PAR> buffer) {
		auto fontDataDataKey = render_orchestrator->MakeDataKey(render_system, u8"global.UI", u8"FontData");

		RenderOrchestrator::BufferWriteKey uiData = render_orchestrator->GetBufferWriteKey(render_system, uiDataDataKey);
		uiData[u8"fontData"][loadedFonts++] = fontDataDataKey;

		RenderOrchestrator::BufferWriteKey fontData = render_orchestrator->GetBufferWriteKey(render_system, fontDataDataKey);

		GTSL::uint32 numberOfGlyphs; buffer >> numberOfGlyphs;

		for(GTSL::uint32 gi = 0; gi < numberOfGlyphs; ++gi) {
			auto glyphReferenceDataKey = render_orchestrator->MakeDataKey(render_system, u8"global.UI", u8"GlyphData");

			fontData[u8"glyphs"][gi] = glyphReferenceDataKey;

			auto glyphReference = render_orchestrator->GetBufferWriteKey(render_system, glyphReferenceDataKey);

			charToGlyphMap.Emplace(FontResourceManager::ALPHABET[gi], gi);
			characters.Emplace(gi, font_data.Characters.array[gi]);

			GTSL::uint32 contourCount; buffer >> contourCount;

			glyphReference[u8"contourCount"] = contourCount;

			for(GTSL::uint32 ci = 0; ci < contourCount; ++ci) {
				GTSL::uint32 pointCount; buffer >> pointCount;

				auto contourReference = glyphReference[u8"contours"][ci];

				GTSL::uint32 linearSegmentCount = 0, quadraticSegmentCount = 0;

				auto linearSegments = contourReference[u8"linearSegments"];
				auto quadraticSegments = contourReference[u8"quadraticSegments"];

				for(GTSL::uint32 pi = 0; pi < pointCount; ++pi) {
					GTSL::uint8 l = 0; buffer >> l;

					if(l == 3) {
						GTSL::Vector2 quadraticSegment[3];
						buffer.Read(8 * 3, reinterpret_cast<byte*>(&quadraticSegment));
						quadraticSegments[quadraticSegmentCount][u8"segments"][0] = quadraticSegment[0];
						quadraticSegments[quadraticSegmentCount][u8"segments"][1] = quadraticSegment[1];
						quadraticSegments[quadraticSegmentCount][u8"segments"][2] = quadraticSegment[2];
						++quadraticSegmentCount;
					} else {
						GTSL::Vector2 linearSegment[2];
						buffer.Read(8 * 2, reinterpret_cast<byte*>(&linearSegment));
						linearSegments[linearSegmentCount][u8"segments"][0] = linearSegment[0];
						linearSegments[linearSegmentCount][u8"segments"][1] = linearSegment[1];
						++linearSegmentCount;
					}
				}

				contourReference[u8"linearSegmentCount"] = linearSegmentCount;
				contourReference[u8"quadraticSegmentCount"] = quadraticSegmentCount;
			}
		}

	}

	RenderModelHandle GetUIMaterial() const { return uiMaterial; }

private:
	RenderOrchestrator::MemberHandle matrixUniformBufferMemberHandle, colorHandle;
	RenderOrchestrator::MemberHandle uiDataStruct;

	RenderOrchestrator::NodeHandle uiMaterialNodeHandle, meshNodeHandle, textMeshNodeHandle, uiInstancesDataNodeHandle, textMaterialNodeHandle;

	GTSL::HashMap<GTSL::uint32, GTSL::uint32, BE::PAR> instancesMap;

	GTSL::uint8 comps = 2;
	RenderModelHandle uiMaterial;

	RenderOrchestrator::DataKeyHandle uiDataDataKey, uiInstancesDataKey;

	GTSL::uint32 loadedFonts = 0;

	GTSL::HashMap<char8_t, GTSL::uint32, BE::PAR> charToGlyphMap;
	GTSL::HashMap<GTSL::uint32, FontResourceManager::Character, BE::PAR> characters;
};

//if (textSystem->GetTexts().ElementCount())
//{
//	int32 atlasIndex = 0;
//	
//	auto& text = textSystem->GetTexts()[0];
//	auto& imageFont = textSystem->GetFont();
//
//	auto x = text.Position.X;
//	auto y = text.Position.Y;
//	
//	byte* data = static_cast<byte*>(info.MaterialSystem->GetRenderGroupDataPointer("TextSystem"));
//
//	GTSL::uint32 offset = 0;
//	
//	GTSL::Matrix4 ortho;
//	auto renderExtent = info.RenderSystem->GetRenderExtent();
//	GTSL::Math::MakeOrthoMatrix(ortho, static_cast<float32>(renderExtent.Width) * 0.5f, static_cast<float32>(renderExtent.Width) * -0.5f, static_cast<float32>(renderExtent.Height) * 0.5f, static_cast<float32>(renderExtent.Height) * -0.5f, 1, 100);
//	GTSL::MemCopy(sizeof(ortho), &ortho, data + offset); offset += sizeof(ortho);
//	GTSL::MemCopy(sizeof(GTSL::uint32), &atlasIndex, data + offset); offset += sizeof(GTSL::uint32); offset += sizeof(GTSL::uint32) * 3;
//	
//	for (auto* c = text.String.begin(); c != text.String.end() - 1; c++)
//	{
//		auto& ch = imageFont.Characters.at(*c);
//
//		float xpos = x + ch.Bearing.X * scale;
//		float ypos = y - (ch.Size.Height - ch.Bearing.Y) * scale;
//
//		float w = ch.Size.Width * scale;
//		float h = ch.Size.Height * scale;
//		
//		// update VBO for each character
//		float vertices[6][4] = {
//			{ xpos,     -(ypos + h),   0.0f, 0.0f },
//			{ xpos,     -(ypos),       0.0f, 1.0f },
//			{ xpos + w, -(ypos),       1.0f, 1.0f },
//
//			{ xpos,     -(ypos + h),   0.0f, 0.0f },
//			{ xpos + w, -(ypos),       1.0f, 1.0f },
//			{ xpos + w, -(ypos + h),   1.0f, 0.0f }
//		};
//		
//		// now advance cursors for next glyph (note that advance is number of 1/64 pixels)
//		x += (ch.Advance >> 6) * scale; // bitshift by 6 to get value in pixels (2^6 = 64)
//
//		GTSL::uint32 val = ch.Position.Width;
//		GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
//		val = ch.Position.Height;
//		GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
//
//		val = ch.Size.Width;
//		GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
//		val = ch.Size.Height;
//		GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
//		
//		for (GTSL::uint32 v = 0; v < 6; ++v)
//		{
//			GTSL::MemCopy(sizeof(GTSL::Vector2), &vertices[v][0], data + offset); offset += sizeof(GTSL::Vector2); //vertices
//			GTSL::MemCopy(sizeof(GTSL::Vector2), &vertices[v][2], data + offset); offset += sizeof(GTSL::Vector2); //uv
//		}
//		
//	}
//
//}

inline auto RenderPassStructToAttachments(const GTSL::Range<const StructElement*> struct_elements) {
	GTSL::StaticVector<RenderOrchestrator::PassData::AttachmentReference, 8> attachmentReferences;

	for(const auto& e : struct_elements) {
		if(e.Type == u8"TextureReference") {
			attachmentReferences.EmplaceBack(GTSL::StringView(e.Name), GTSL::StringView(e.Name), GAL::AccessTypes::READ);
		}

		if(e.Type == u8"ImageReference") {
			attachmentReferences.EmplaceBack(GTSL::StringView(e.Name), GTSL::StringView(e.Name), GAL::AccessTypes::WRITE);
		}
	}

	return attachmentReferences;
}