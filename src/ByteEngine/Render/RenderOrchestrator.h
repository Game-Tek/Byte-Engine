#pragma once

#include "ByteEngine/Game/System.hpp"

#include <GTSL/Vector.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/PagedVector.h>
#include <GTSL/SparseVector.hpp>
#include <GTSL/Tree.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Bitfield.h>

#include "ByteEngine/Id.h"
#include "RenderSystem.h"
#include "RenderTypes.h"
#include "StaticMeshRenderGroup.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/Tasks.h"
#include "ByteEngine/Resources/ShaderResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"
#include "Culling.h"
#include "LightsRenderGroup.h"
#include "UIManager.h"
#include "ByteEngine/MetaStruct.hpp"

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
struct TypeNamer<float32> {
	static constexpr const char8_t* NAME = u8"float32";
};

template<>
struct TypeNamer<GTSL::Matrix3x4> {
	static constexpr const char8_t* NAME = u8"matrix3x4f";
};

class RenderManager : public BE::System
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
	MAKE_HANDLE(uint32, ElementData);

	enum class PassType : uint8 {
		RASTER, COMPUTE, RAY_TRACING
	};

	enum class NodeType : uint8 {
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

		ElementDataHandle Handle; uint32 Index = 0;
	};

	struct NodeHandle {
		NodeHandle() = default;
		NodeHandle(const uint32 val) : value(val) {}

		uint32 operator()() const { return value; }

		operator bool() const { return value; }
	private:
		uint32 value = 0;
	};

	//MAKE_HANDLE(uint32, DataKey);

protected:
	MAKE_HANDLE(uint64, Resource);

	struct AttachmentData {
		Id Name;
		GAL::TextureLayout Layout;
		GAL::PipelineStage ConsumingStages;
		GAL::AccessType Access;
	};

	struct APIRenderPassData {
		RenderPass RenderPass;
		uint8 APISubPass = 0, SubPassCount = 0;
	};

public:
	struct MemberInfo : Member {
		MemberInfo() = default;
		MemberInfo(MemberHandle* memberHandle, GTSL::StringView type, GTSL::StringView name) : Member(type, name), Handle(memberHandle) {}
		MemberInfo(MemberHandle* memberHandle, GTSL::Range<MemberInfo*> memberInfos, GTSL::StringView type, GTSL::StringView name, const uint32 alignment = 0) : Member(type, name), Handle(memberHandle), MemberInfos(memberInfos), alignment(alignment) {}

		MemberHandle* Handle = nullptr;
		GTSL::Range<MemberInfo*> MemberInfos;
		uint16 alignment = 1;
	};

	explicit RenderOrchestrator(const InitializeInfo& initializeInfo);

	MAKE_HANDLE(uint32, Set);

	struct SubSetDescription {
		SetHandle SetHandle; uint32 Subset;
		GAL::BindingType Type;
	};

	MAKE_HANDLE(SubSetDescription, SubSet);
	MAKE_HANDLE(uint64, SetLayout);
	MAKE_HANDLE(uint32, DataKey);

	DataKeyHandle MakeDataKey() {
		auto pos = dataKeys.GetLength();
		dataKeys.EmplaceBack();
		return DataKeyHandle(pos);
	}

	DataKeyHandle MakeDataKey(RenderSystem* render_system, RenderSystem::BufferHandle buffer_handle, const DataKeyHandle data_key_handle = DataKeyHandle()) {
		if (data_key_handle) {
			auto& dataKey = dataKeys[data_key_handle()];
			dataKey.Buffer = buffer_handle;
			UpdateDataKey(data_key_handle);
			return data_key_handle;
		}

		auto pos = dataKeys.GetLength();
		auto& dataKey = dataKeys.EmplaceBack();
		dataKey.Buffer = buffer_handle;
		return DataKeyHandle(pos);
	}

	//DataKeyHandle MakeDataKey(MemberHandle memberHandle) {
	//	auto offset = renderDataOffset;
	//	renderDataOffset += memberHandle.Size;
	//	auto pos = dataKeys.GetLength();
	//	dataKeys.EmplaceBack(offset);
	//	return DataKeyHandle(pos);
	//}

	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo, RenderSystem* renderSystem);

	//HACKS, REMOVE
	NodeHandle GetGlobalDataLayer() const { return globalData; }
	NodeHandle GetCameraDataLayer() const { return cameraDataNode; }
	//HACKS, REMOVE

	[[nodiscard]] ShaderGroupHandle CreateShaderGroup(Id shader_group_name);

	void AddAttachment(Id attachmentName, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type);

	DataKeyHandle GetIndex(const DataKeyHandle& data_key_handle, uint32 index) {
		const auto& dataKey = dataKeys[data_key_handle()];
		auto newDataKeyIndex = dataKeys.GetLength();
		auto& newDataKey = dataKeys.EmplaceBack();
		newDataKey.Offset = dataKey.Offset + GetSize(dataKey.Handle, true) * index;
		newDataKey.Handle = dataKey.Handle;
		newDataKey.Buffer = dataKey.Buffer;
		return DataKeyHandle(newDataKeyIndex);
	}

	void AddInstance(NodeHandle node_handle) {
		++getPrivateNode<MeshData>(node_handle).InstanceCount;
	}

	NodeHandle AddVertexBufferBind(RenderSystem* render_system, NodeHandle parent_node_handle, RenderSystem::BufferHandle buffer_handle, GTSL::Range<const GTSL::Range<const GAL::ShaderDataType*>*> meshVertexLayout) {
		auto nodeHandle = addInternalNode<VertexBufferBindData>(0, parent_node_handle);
		auto& node = getPrivateNode<VertexBufferBindData>(nodeHandle);
		node.Handle = buffer_handle; node.VertexCount = 0; node.VertexSize = 0;

		{
			uint32 offset = 0;

			for (auto& i : meshVertexLayout) {
				node.VertexSize += GAL::GraphicsPipeline::GetVertexSize(i);
			}

			auto bufferSize = render_system->GetBufferRange(buffer_handle).Bytes();

			for (auto& i : meshVertexLayout) {
				node.Offsets.EmplaceBack(offset);
				offset += GAL::GraphicsPipeline::GetVertexSize(i) * (bufferSize / node.VertexSize);
			}
		}


		return nodeHandle;
	}

	void AddVertices(const NodeHandle node_handle, uint32 count) {
		auto& node = getPrivateNode<VertexBufferBindData>(node_handle);
		node.VertexCount += count;
	}

	NodeHandle AddIndexBufferBind(NodeHandle parent_node_handle, RenderSystem::BufferHandle buffer_handle) {
		auto nodeHandle = addInternalNode<IndexBufferBindData>(0, parent_node_handle);
		auto& node = getPrivateNode<IndexBufferBindData>(nodeHandle);
		node.BufferHandle = buffer_handle; node.IndexCount = 0; node.IndexType = GAL::IndexType::UINT16;
		return nodeHandle;
	}

	void AddIndices(const NodeHandle node_handle, uint32 count) {
		auto& node = getPrivateNode<IndexBufferBindData>(node_handle);
		node.IndexCount += count;
	}

	GTSL::Delegate<void(RenderOrchestrator*, RenderSystem*)> shaderGroupNotify;
	DataKeyHandle globalDataDataKey, cameraDataKeyHandle;

	void AddNotifyShaderGroupCreated(GTSL::Delegate<void(RenderOrchestrator*, RenderSystem*)> notify_delegate) {
		shaderGroupNotify = notify_delegate;
	}

	struct PassData {
		struct AttachmentReference {
			GAL::AccessType Access; Id Name;
		};
		GTSL::StaticVector<AttachmentReference, 8> Attachments;

		PassType PassType;
	};
	NodeHandle AddRenderPass(GTSL::StringView renderPassName, NodeHandle parent_node_handle, RenderSystem* renderSystem, PassData passData, ApplicationManager* am);

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

	MemberHandle CreateMember2(GTSL::StringView parents, GTSL::StringView structName, const GTSL::Range<const GTSL::Pair<GTSL::ShortString<32>, GTSL::ShortString<32>>*> members) {
		GTSL::StaticVector<MemberInfo, 16> mem;

		for(auto& e : members) {
			mem.EmplaceBack(nullptr, e.First, e.Second);
		}

		return  CreateMember(parents, structName, mem);
	}

	MemberHandle CreateMember(GTSL::StringView parents, GTSL::StringView structName, const GTSL::Range<MemberInfo*> members) {
		auto parseMembers = [&](auto&& self, GTSL::StringView par, GTSL::StringView typeName, GTSL::StringView name, GTSL::Range<MemberInfo*> levelMembers, uint16 level) -> ElementDataHandle {
			auto currentScope = GTSL::StaticString<128>(par) << u8"." << typeName;

			auto dataTypeEmplace = tryAddElement(par, typeName, ElementData::ElementType::TYPE);

			if(dataTypeEmplace.State() == 1) { //when element already exists clear data to redeclare element
				auto& e = getElement(dataTypeEmplace.Get());
				e.TyEl.Size = 0;
			}

			//if (name != u8"root") {
			//	addMember(par, typeName, name);
			//}

			uint32 offset = 0;

			for (uint8 m = 0; m < levelMembers.ElementCount(); ++m) {
				auto& member = levelMembers[m];

				ElementDataHandle handle;

				if (member.MemberInfos.ElementCount()) {
					handle = self(self, currentScope, member.Type, member.Name, levelMembers[m].MemberInfos, level + 1);
					getElement(handle).TyEl.Alignment = 64;
				}

				handle = addMember(currentScope, member.Type, member.Name).Get();

				if (handle) {
					offset = GTSL::Math::RoundUpByPowerOf2(offset, static_cast<uint32>(member.alignment));

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

	NodeHandle AddMaterial(RenderSystem* render_system, NodeHandle parent_node_handle, ShaderGroupHandle materialHandle) {
		auto& material = shaderGroups[materialHandle.ShaderGroupIndex];
		auto pipelineBindNode = addPipelineBindNode(parent_node_handle, materialHandle);
		auto& materialNode = getNode(pipelineBindNode);
		setNodeName(pipelineBindNode, shaderGroups[materialHandle.ShaderGroupIndex].Name);
		return pipelineBindNode;
	}

	NodeHandle AddDataNode(Id layerName, NodeHandle parent, const MemberHandle member_handle) {
		auto dataKeyHandle = DataKeyHandle(dataKeys.GetLength());
		auto& dataKey = dataKeys.EmplaceBack();
		dataKey.Buffer = renderBuffers[0].BufferHandle;
		dataKey.Offset = renderDataOffset;
		dataKey.Handle = member_handle.Handle;
		renderDataOffset += GetSize(member_handle.Handle);
		auto nodeHandle = addInternalNode<LayerData>(~0ull, parent);

		dataKey.Nodes.EmplaceBack(nodeHandle);
		UpdateDataKey(dataKeyHandle);
		getNode(nodeHandle).Name = GTSL::StringView(layerName);

		return nodeHandle;
	}

	RenderSystem::CommandListHandle graphicsCommandLists[MAX_CONCURRENT_FRAMES];
	RenderSystem::CommandListHandle buildCommandList[MAX_CONCURRENT_FRAMES], transferCommandList[MAX_CONCURRENT_FRAMES];

	NodeHandle AddMesh(const NodeHandle parentNodeHandle, uint32 meshId, uint32 indexCount, uint32 vertexOffset = 0) {
		auto nodeHandle = addInternalNode<MeshData>(meshId, parentNodeHandle);
		//SetNodeState(nodeHandle, false);
		getNode(nodeHandle).Name = GTSL::ShortString<32>(u8"Render Mesh");
		auto& node = getPrivateNode<MeshData>(nodeHandle);
		node.IndexCount = indexCount;
		node.VertexOffset = vertexOffset;
		return nodeHandle;
	}

	//void AddMesh(NodeHandle node_handle, RenderSystem::BufferHandle meshHandle, uint32 vertexCount, uint32 vertexSize, uint32 indexCount, GAL::IndexType indexType, GTSL::Range<const GTSL::Range<const GAL::ShaderDataType*>*> meshVertexLayout) {
	//	bool foundLayout = false; uint8 layoutIndex = 0;
	//
	//	for (; layoutIndex < vertexLayouts.GetLength(); ++layoutIndex) {
	//		if (vertexLayouts[layoutIndex].GetLength() != meshVertexLayout.ElementCount()) { continue; }
	//
	//		foundLayout = true;
	//
	//		for (uint8 i = 0; i < meshVertexLayout.ElementCount(); ++i) {
	//			if (meshVertexLayout[i] != vertexLayouts[layoutIndex][i]) { foundLayout = false; break; }
	//		}
	//
	//		if (foundLayout) { break; }
	//
	//		++layoutIndex;
	//	}
	//
	//	if (!foundLayout) {
	//		foundLayout = true;
	//		layoutIndex = vertexLayouts.GetLength();
	//		auto& vertexLayout = vertexLayouts.EmplaceBack();
	//
	//		for (auto e : meshVertexLayout) {
	//			vertexLayout.EmplaceBack(e);
	//		}
	//	}
	//
	//	//auto& meshNode = getPrivateNode<MeshData>(node_handle);
	//	//meshNode.IndexCount = indexCount;
	//	//meshNode.IndexType = indexType;
	//	//meshNode.VertexCount = vertexCount;
	//	//meshNode.VertexSize = vertexSize;
	//	//
	//	//{
	//	//	uint32 offset = 0;
	//	//
	//	//	for(auto& i : meshVertexLayout) {
	//	//		auto vertexSize = GAL::GraphicsPipeline::GetVertexSize(i);
	//	//
	//	//		meshNode.Offsets.EmplaceBack(offset);
	//	//
	//	//		offset += vertexSize * vertexCount;
	//	//	}
	//	//}
	//
	//	SetNodeState(node_handle, true);
	//}

	template<typename T>
	void addPendingWrite(const T& val, RenderSystem::BufferHandle buffer_handle, byte* writeTo, byte* readFrom, uint32 offset, uint8 current_frame, uint8 next_frame) {
		auto key = uint64(buffer_handle()) << 32 | offset;

		if (pendingWrites.Find(key)) {
			pendingWrites.Remove(key);
		}

		auto& write = pendingWrites.Emplace(key);
		write.Size = sizeof(T);

		write.WhereToWrite = writeTo + offset;
		write.FrameCountdown.Set(next_frame, true);

		if (readFrom) {
			write.WhereToRead = readFrom + offset;
		}
		else {
			*reinterpret_cast<T*>(buffer[0] + offsets[0]) = val;
			write.WhereToRead = buffer[0] + offsets[0];
			offsets[0] += sizeof(T);
		}
	}

	struct BufferWriteKey {
		uint32 Offset = 0;
		RenderSystem* render_system = nullptr; RenderOrchestrator* render_orchestrator = nullptr;
		uint8 Frame = 0, NextFrame = 0;
		RenderSystem::BufferHandle buffer_handle;
		GTSL::StaticString<256> Path{ u8"global" };
		ElementDataHandle ElementHandle;

		BufferWriteKey() {

		}

		BufferWriteKey(const BufferWriteKey&) = default;
		BufferWriteKey(uint32 newOffset, GTSL::StringView path, const ElementDataHandle element_data_handle, const BufferWriteKey& other) : BufferWriteKey(other) { Offset = newOffset; Path = path; ElementHandle = element_data_handle; }

		//BufferWriteKey operator[](const MemberHandle member_handle) {
		//	return BufferWriteKey{ Offset + member_handle.Offset, Path, *this };
		//}

		BufferWriteKey operator[](const uint32 index) {
			auto& e = render_orchestrator->getElement(ElementHandle);

			BE_ASSERT(render_orchestrator->getElement(ElementHandle).Type == ElementData::ElementType::MEMBER);
			if(e.Mem.Multiplier == 1) {
				render_orchestrator->getLogger()->PrintObjectLog(render_orchestrator, BE::Logger::VerbosityLevel::FATAL, u8"Tried to access ", Path, u8" as array but it isn't.");
				return BufferWriteKey{ 0xFFFFFFFF, Path, ElementHandle, *this };
			}

			if(index >= e.Mem.Multiplier) {
				render_orchestrator->getLogger()->PrintObjectLog(render_orchestrator, BE::Logger::VerbosityLevel::FATAL, u8"Tried to access index", index, u8" of ", Path, u8" but array size is ", e.Mem.Multiplier);
				return BufferWriteKey{ 0xFFFFFFFF, Path, ElementHandle, *this };
			}

			return BufferWriteKey{ Offset + render_orchestrator->GetSize(ElementHandle, true) * index, Path, ElementHandle, *this };
		}

		BufferWriteKey operator[](const GTSL::StringView path) {
			auto newPath = Path; newPath << u8"." << path;
			if(auto r = render_orchestrator->GetRelativeOffset(ElementHandle, path)) {
				return BufferWriteKey{ Offset + r.Get().Second, newPath, r.Get().First, *this };
			} else {
				render_orchestrator->getLogger()->PrintObjectLog(render_orchestrator, BE::Logger::VerbosityLevel::FATAL, u8"Tried to access ", Path, u8" while writing, which doesn't exist.");
				return BufferWriteKey{ 0xFFFFFFFF, Path, ElementHandle, *this };
			}
		}

		template<typename T>
		const BufferWriteKey& operator=(const T& obj) const {
			if (Offset == ~0u or !validateType<T>()) { return *this; }
			*reinterpret_cast<T*>(render_system->GetBufferPointer(buffer_handle, Frame) + Offset) = obj;
			render_orchestrator->addPendingWrite(obj, buffer_handle, render_system->GetBufferPointer(buffer_handle, NextFrame), render_system->GetBufferPointer(buffer_handle, Frame), Offset, Frame, NextFrame);
			return *this;
		}

		const BufferWriteKey& operator=(const RenderSystem::AccelerationStructureHandle acceleration_structure_handle) const {
			if (Offset == ~0u or !validateType<RenderSystem::AccelerationStructureHandle>()) { return *this; }
			*reinterpret_cast<GAL::DeviceAddress*>(render_system->GetBufferPointer(buffer_handle, Frame) + Offset) = render_system->GetTopLevelAccelerationStructureAddress(acceleration_structure_handle, render_system->GetCurrentFrame());
			render_orchestrator->addPendingWrite(render_system->GetTopLevelAccelerationStructureAddress(acceleration_structure_handle, NextFrame), buffer_handle, render_system->GetBufferPointer(buffer_handle, NextFrame), nullptr, Offset, Frame, NextFrame);
			return *this;
		}
		
		const BufferWriteKey& operator=(const RenderSystem::BufferHandle obj) const {
			if (Offset == ~0u or !validateType<RenderSystem::BufferHandle>()) { return *this; }
			*reinterpret_cast<GAL::DeviceAddress*>(render_system->GetBufferPointer(buffer_handle, Frame) + Offset) = render_system->GetBufferAddress(obj);
			render_orchestrator->addPendingWrite(render_system->GetBufferAddress(obj, NextFrame, true), buffer_handle, render_system->GetBufferPointer(buffer_handle, NextFrame), nullptr, Offset, Frame, NextFrame);
			return *this;
		}

		const BufferWriteKey& operator=(const DataKeyHandle obj) const {
			return (*this).operator=(render_orchestrator->dataKeys[obj()].Buffer);
		}

		template<typename T>
		bool validateType() const {
			auto name = TypeNamer<T>::NAME;

			if(name) {
				if(render_orchestrator->getElement(render_orchestrator->getElement(ElementHandle).Mem.TypeHandle).Name == name) {
					return true;
				}

				render_orchestrator->getLogger()->PrintObjectLog(render_orchestrator, BE::Logger::VerbosityLevel::FATAL, u8"Tried to access ", Path, u8" while writing, but types don't match.");
				return false;
			}

			return true;
		}
	};

	[[nodiscard]] NodeHandle AddDataNode(const NodeHandle parent_node_handle, const GTSL::StringView node_name, const DataKeyHandle data_key_handle) {
		auto& dataKey = dataKeys[data_key_handle()];
		auto nodeHandle = addInternalNode<LayerData>(0, parent_node_handle);
		dataKey.Nodes.EmplaceBack(nodeHandle);
		UpdateDataKey(data_key_handle);
		setNodeName(nodeHandle, node_name);
		return nodeHandle;
	}

	void UpdateDataKey(const DataKeyHandle data_key_handle) {
		auto& dataKey = dataKeys[data_key_handle()];

		for (auto& e : dataKey.Nodes) {
			//if (dataKey.Buffer) {
			//	getInternalNodeFromPublicHandle(e).Address[0] = renderSystem->GetBufferAddress(dataKey.Buffer, 0, true);
			//	getInternalNodeFromPublicHandle(e).Address[1] = renderSystem->GetBufferAddress(dataKey.Buffer, 1, true);
			//}
			//
			//getInternalNodeFromPublicHandle(e).BufferHandle = dataKey.Buffer;
			//getInternalNodeFromPublicHandle(e).Offset = dataKey.Offset;
			SetNodeState(e, static_cast<bool>(dataKey.Buffer));
			getPrivateNode<LayerData>(e).DataKey = data_key_handle;
		}
	}
	
	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const NodeHandle node_handle, uint32 frame = ~0u) {
		auto& node = getPrivateNode<LayerData>(node_handle);
		BufferWriteKey buffer_write_key;
		buffer_write_key.render_system = render_system;
		buffer_write_key.render_orchestrator = this;
		buffer_write_key.Frame = render_system->GetCurrentFrame();
		buffer_write_key.NextFrame = (render_system->GetCurrentFrame() + 1) % render_system->GetPipelinedFrames();
		const auto& dataKey = dataKeys[node.DataKey()];
		buffer_write_key.buffer_handle = dataKey.Buffer;
		buffer_write_key.Offset = dataKey.Offset;
		buffer_write_key.ElementHandle = dataKey.Handle;
		return buffer_write_key;
	}

	BufferWriteKey GetBufferWriteKey(RenderSystem* render_system, const DataKeyHandle data_key_handle) {
		const auto& dataKey = dataKeys[data_key_handle()];
		render_system->SignalBufferWrite(dataKey.Buffer);
		BufferWriteKey buffer_write_key;
		buffer_write_key.render_system = render_system;
		buffer_write_key.render_orchestrator = this;
		if (render_system->IsUpdatable(dataKey.Buffer)) {
			buffer_write_key.Frame = render_system->GetCurrentFrame();
			buffer_write_key.NextFrame = (render_system->GetCurrentFrame() + 1) % render_system->GetPipelinedFrames();
		}
		else {
			buffer_write_key.Frame = 0;
			buffer_write_key.NextFrame = 0;
		}
		buffer_write_key.buffer_handle = dataKey.Buffer;
		buffer_write_key.ElementHandle = dataKey.Handle;
		return buffer_write_key;
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
		auto& set = sets[setHandle()];
		commandBuffer.BindBindingsSets(renderSystem->GetRenderDevice(), shaderStage, GTSL::Range<BindingsSet*>(1, &set.BindingsSet[renderSystem->GetCurrentFrame()]), set.PipelineLayout, set.Level);
	}

	void WriteBinding(const RenderSystem* renderSystem, SubSetHandle setHandle, RenderSystem::TextureHandle textureHandle, uint32 bindingIndex, uint8 frameIndex) {
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
		info.TextureLayout = layout;
		info.FormatDescriptor;

		descriptorsUpdates[frameIndex].AddTextureUpdate(setHandle, bindingIndex, info);
	}

	enum class SubSetType : uint8 {
		BUFFER, READ_TEXTURES, WRITE_TEXTURES, RENDER_ATTACHMENT, ACCELERATION_STRUCTURE, SAMPLER
	};

	struct SubSetDescriptor {
		SubSetType SubSetType; uint32 BindingsCount;
		SubSetHandle* Handle;
		GTSL::Range<const TextureSampler*> Sampler;
	};
	SetLayoutHandle AddSetLayout(RenderSystem* renderSystem, SetLayoutHandle parentName, const GTSL::Range<SubSetDescriptor*> subsets) {
		uint64 hash = quickhash64(GTSL::Range(subsets.Bytes(), reinterpret_cast<const byte*>(subsets.begin())));

		SetLayoutHandle parentHandle;
		uint32 level;

		if (parentName()) {
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

			for (uint8 i = 0; i < level; ++i) { bindingsSetLayouts.EmplaceBack(); }

			for (uint8 i = 0, l = level - 1; i < level; ++i, --l) {
				bindingsSetLayouts[l] = setLayoutDatas[lastSet()].BindingsSetLayout;
				lastSet = setLayoutDatas[lastSet()].Parent;
			}
		}

		setLayoutData.Stage = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::COMPUTE;

		GTSL::StaticVector<BindingsSetLayout::BindingDescriptor, 10> subSetDescriptors;

		for (const auto& e : subsets) {
			BindingsSetLayout::BindingDescriptor binding_descriptor;

			if (e.BindingsCount != 1) { binding_descriptor.Flags = GAL::BindingFlags::PARTIALLY_BOUND; }
			binding_descriptor.BindingsCount = e.BindingsCount;

			switch (e.SubSetType) {
			case SubSetType::BUFFER: binding_descriptor.BindingType = GAL::BindingType::STORAGE_BUFFER; break;
			case SubSetType::READ_TEXTURES: binding_descriptor.BindingType = GAL::BindingType::SAMPLED_IMAGE; break;
			case SubSetType::WRITE_TEXTURES: binding_descriptor.BindingType = GAL::BindingType::STORAGE_IMAGE; break;
			case SubSetType::RENDER_ATTACHMENT: binding_descriptor.BindingType = GAL::BindingType::INPUT_ATTACHMENT; break;
			case SubSetType::SAMPLER: {
				binding_descriptor.BindingType = GAL::BindingType::SAMPLER;
				binding_descriptor.Samplers = e.Sampler;
				binding_descriptor.BindingsCount = e.Sampler.ElementCount();
				break;
			}
			case SubSetType::ACCELERATION_STRUCTURE:
				binding_descriptor.BindingType = GAL::BindingType::ACCELERATION_STRUCTURE;
				binding_descriptor.ShaderStage = GAL::ShaderStages::RAY_GEN;
				break;
			}

			binding_descriptor.ShaderStage = setLayoutData.Stage;

			subSetDescriptors.EmplaceBack(binding_descriptor);
		}

		setLayoutData.BindingsSetLayout.Initialize(renderSystem->GetRenderDevice(), subSetDescriptors);
		bindingsSetLayouts.EmplaceBack(setLayoutData.BindingsSetLayout);

		GAL::PushConstant pushConstant;
		pushConstant.Stage = setLayoutData.Stage;
		pushConstant.NumberOf4ByteSlots = 32;
		setLayoutData.PipelineLayout.Initialize(renderSystem->GetRenderDevice(), &pushConstant, bindingsSetLayouts);

		return SetLayoutHandle(hash);
	}

	SetHandle AddSet(RenderSystem* renderSystem, Id setName, SetLayoutHandle setLayoutHandle, const GTSL::Range<SubSetDescriptor*> setInfo) {
		GTSL::StaticVector<BindingsSetLayout::BindingDescriptor, 16> bindingDescriptors;

		for (auto& ss : setInfo) {
			GAL::ShaderStage enabledShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::COMPUTE;

			switch (ss.SubSetType) {
			case SubSetType::BUFFER:
				bindingDescriptors.EmplaceBack(GAL::BindingType::STORAGE_BUFFER, enabledShaderStages, ss.BindingsCount, GAL::BindingFlags::PARTIALLY_BOUND);
				break;
			case SubSetType::READ_TEXTURES:
				bindingDescriptors.EmplaceBack(GAL::BindingType::SAMPLED_IMAGE, enabledShaderStages, ss.BindingsCount, GAL::BindingFlags::PARTIALLY_BOUND);
				break;
			case SubSetType::WRITE_TEXTURES:
				bindingDescriptors.EmplaceBack(GAL::BindingType::STORAGE_IMAGE, enabledShaderStages, ss.BindingsCount, GAL::BindingFlags::PARTIALLY_BOUND);
				break;
			case SubSetType::RENDER_ATTACHMENT:
				bindingDescriptors.EmplaceBack(GAL::BindingType::INPUT_ATTACHMENT, enabledShaderStages, ss.BindingsCount, GAL::BindingFlags::PARTIALLY_BOUND);
				break;
			case SubSetType::ACCELERATION_STRUCTURE:
				bindingDescriptors.EmplaceBack(GAL::BindingType::ACCELERATION_STRUCTURE, enabledShaderStages, ss.BindingsCount, GAL::BindingFlag());
				break;
			case SubSetType::SAMPLER:
				bindingDescriptors.EmplaceBack(GAL::BindingType::SAMPLER, enabledShaderStages, ss.BindingsCount, GAL::BindingFlag());
				break;
			default:;
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

	[[nodiscard]] DataKeyHandle CreateDataKey(RenderSystem* renderSystem, const GTSL::StringView scope, const GTSL::StringView type, DataKeyHandle data_key_handle = DataKeyHandle(), GAL::BufferUse buffer_uses = GAL::BufferUse()) {
		RenderSystem::BufferHandle b;

		GTSL::StaticString<64> string(u8"Buffer: "); string << scope << type;
		auto handle = addMember(scope, type, string);

		auto size = GetSize(handle.Get());

		b = renderSystem->CreateBuffer(size, buffer_uses, true, true, b);
		auto dataKeyHandle = MakeDataKey(renderSystem, b, data_key_handle);
		dataKeys[dataKeyHandle()].Handle = handle.Get();
		return dataKeyHandle;
	}

	struct BindingsSetData {
		BindingsSetLayout BindingsSetLayout;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		uint32 DataSize = 0;
	};

	void SetNodeState(const NodeHandle layer_handle, const bool state) { //TODO: DONT CHANGE STATE IF THERE ARE PENDING RESOURCES WHICH SHOULD IMPEDE ENABLING THE NODE
		renderingTree.ToggleBranch(layer_handle(), state);
	}

	NodeHandle GetSceneReference() const {
		return NodeHandle(); //todo: gen
	}

	bool GetResourceState(const ResourceHandle resource_handle) {
		if (!resources.Find(resource_handle())) { BE_LOG_WARNING(u8"Tried to get resource handle state for invalid handle."); return false; }
		return resources[resource_handle()].isValid();
	}

	ResourceHandle GetResourceForShaderGroup(const Id shaderGroup) const {
		if (!shaderGroupsByName.Find(shaderGroup)) { return ResourceHandle(); }
		return shaderGroups[shaderGroupsByName[shaderGroup]].ResourceHandle;
	}

	void PrintMember(const DataKeyHandle data_key_handle, RenderSystem* render_system) const {
		byte* beginPointer;

		GTSL::StaticString<4096> string(u8"\n"); //start struct on new line, looks better when printed

		const auto& dataKey = dataKeys[data_key_handle()];
		const uint32 startOffset = dataKey.Offset;

		auto walkTree = [&](const ElementDataHandle member_handle, uint32 level, uint32 offset, auto&& self) -> uint32 {
			auto& e = elements[member_handle()];
			auto& dt = getElement(e.Mem.TypeHandle);

			for (uint32 t = 0; t < e.Mem.Multiplier; ++t) {
				string += u8"\n";

				for (uint32 i = 0; i < level; ++i) { string += U'	'; } //insert tab for every space deep we are to show struct depth

				string += u8"offset: "; ToString(string, offset - startOffset); string += u8", "; string += dt.DataType; string += u8" ";
				if(e.Mem.Multiplier > 1) {
					string += '['; GTSL::ToString(string, t); string += u8"] ";
				}
				string += e.Name; string += u8": ";

				if (FindLast(dt.DataType, U'*')) {
					GTSL::ToString(string, reinterpret_cast<uint64*>(beginPointer + offset)[0]);
				}
				else {
					switch (GTSL::Hash(dt.DataType)) {
					case GTSL::Hash(u8"ptr_t"): {
						GTSL::ToString(string, reinterpret_cast<uint64*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"uint32"): {
						GTSL::ToString(string, reinterpret_cast<uint32*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"uint64"): {
						GTSL::ToString(string, reinterpret_cast<uint64*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"float32"): {
						GTSL::ToString(string, reinterpret_cast<float32*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"TextureReference"): {
						GTSL::ToString(string, reinterpret_cast<uint32*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"ImageReference"): {
						GTSL::ToString(string, reinterpret_cast<uint32*>(beginPointer + offset)[0]);
						break;
					}
					case GTSL::Hash(u8"matrix4f"): {
						auto matrixPointer = reinterpret_cast<GTSL::Matrix4*>(beginPointer + offset)[0];

						for (uint8 r = 0; r < 4; ++r) {
							for (uint32 i = 0; i < level && r; ++i) { string += U'	'; } //insert tab for every space deep we are to show struct depth

							for (uint8 c = 0; c < 4; ++c) {
								GTSL::ToString(string, matrixPointer[r][c]); string += u8" ";
							}

							string += U'\n';
						}

						break;
					}
					case GTSL::Hash(u8"ShaderHandle"): {

						for (uint32 i = 0; i < 4; ++i) {
							uint64 val = reinterpret_cast<uint64*>(beginPointer + offset)[i];
							if (i) { string << u8"-"; } ToString(string, val);
						}

						uint64 shaderHandleHash = quickhash64({ 32, reinterpret_cast<byte*>(beginPointer + offset) });

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

				uint32 size = 0;

				for (auto& e : dt.children) {
					if (getElement(e.Handle).Type == ElementData::ElementType::MEMBER) {
						size = GTSL::Math::RoundUpByPowerOf2(size, getElement(getElement(ElementDataHandle(e.Handle)).Mem.TypeHandle).TyEl.Alignment);
						size += self(e.Handle, level + 1, offset + size, self);
					}
				}

				offset += dt.TyEl.Size;

				BE_ASSERT(dt.Type == ElementData::ElementType::TYPE);
			}

			return dt.TyEl.Size * e.Mem.Multiplier; //todo: align
		};

		if (render_system->IsUpdatable(dataKey.Buffer)) {
			string += u8"Frame: 0\n";
			beginPointer = render_system->GetBufferPointer(dataKey.Buffer, 0);
			walkTree(ElementDataHandle(dataKey.Handle), 0, startOffset, walkTree);
			string.Resize(0);
			string += u8"\nFrame: 1\n";
			beginPointer = render_system->GetBufferPointer(dataKey.Buffer, 1);
			walkTree(ElementDataHandle(dataKey.Handle), 0, startOffset, walkTree);
		} else {
			beginPointer = render_system->GetBufferPointer(dataKey.Buffer, 0);
			walkTree(ElementDataHandle(dataKey.Handle), 0, startOffset, walkTree);
		}

		BE_LOG_MESSAGE(string);
	}

	GAL::DeviceAddress GetAddress(RenderSystem* render_system, const DataKeyHandle data_key_handle) const {
		return render_system->GetBufferAddress(dataKeys[data_key_handle()].Buffer);
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
	NodeHandle rayTraceNode;

	SubSetHandle renderGroupsSubSet;
	SubSetHandle renderPassesSubSet;

	MemberHandle cameraMatricesHandle;
	MemberHandle globalDataHandle;
	SubSetHandle textureSubsetsHandle;
	SubSetHandle imagesSubsetHandle;
	SubSetHandle samplersSubsetHandle;

	RenderSystem::WorkloadHandle graphicsWorkloadHandle[MAX_CONCURRENT_FRAMES], buildAccelerationStructuresWorkloadHandle[MAX_CONCURRENT_FRAMES], imageAcquisitionWorkloadHandles[MAX_CONCURRENT_FRAMES];

	GTSL::HashMap<Id, uint32, BE::PAR> rayTracingSets;

	GTSL::StaticVector<GTSL::StaticVector<GTSL::StaticVector<GAL::ShaderDataType, 8>, 8>, 16> vertexLayouts;

	GTSL::HashMap<uint64, GTSL::StaticString<128>, BE::PAR> shaderHandlesDebugMap;

	struct RenderState {
		GAL::ShaderStage ShaderStages;
		uint8 streamsCount = 0, buffersCount = 0;

		uint32 BoundPipelineIndex;

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

	struct ShaderData {
		GAL::VulkanShader Shader;
		GAL::ShaderType Type;
		GTSL::StaticString<64> Name;
	};
	GTSL::HashMap<uint64, ShaderData, BE::PAR> shaders;

	struct MeshData {
		uint32 InstanceCount = 0;
		uint32_t IndexCount, VertexOffset;
	};

	struct DispatchData {
		GTSL::Extent3D DispatchSize;
	};

	struct PipelineBindData {
		ShaderGroupHandle Handle;
	};

	struct RayTraceData {
		uint32 ShaderGroupIndex = 0xFFFFFFFF;
	};

	struct RenderPassData {
		PassType Type;
		GTSL::StaticVector<AttachmentData, 4> Attachments;
		GAL::PipelineStage PipelineStages;
		MemberHandle RenderTargetReferences;
		ResourceHandle ResourceHandle;

		RenderPassData() : Type(PassType::RASTER), Attachments(), PipelineStages() {
		}

		//union {
		//	APIRenderPassData APIRenderPass;
		//};
	};

	struct LayerData {
		DataKeyHandle DataKey;
	};

	struct PublicNode {
		GTSL::ShortString<32> Name;
		NodeType Type; uint8 Level = 0;
		uint32 InstanceCount = 0;
	};

	//Node's names are nnot provided inn the CreateNode functions since we donn't wantt to generate debug nnames in realease builds, and the compiler won't eliminnate the useless stringg generation code
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

	NodeHandle globalData, cameraDataNode;

	void transitionImages(CommandList commandBuffer, RenderSystem* renderSystem, const RenderPassData* internal_layer);

	struct ShaderLoadInfo {
		ShaderLoadInfo() = default;
		ShaderLoadInfo(const BE::PAR& allocator) noexcept : Buffer(allocator), MaterialIndex(0) {}
		ShaderLoadInfo(ShaderLoadInfo&& other) noexcept : Buffer(MoveRef(other.Buffer)), MaterialIndex(other.MaterialIndex), handle(other.handle) {}
		GTSL::Buffer<BE::PAR> Buffer; uint32 MaterialIndex;
		NodeHandle handle;
	};

	uint64 resourceCounter = 0;

	ResourceHandle makeResource() {
		resources.Emplace(++resourceCounter);
		return ResourceHandle(resourceCounter);
	}

	void BindToNode(NodeHandle node_handle, ResourceHandle resource_handle) {
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
		GTSL::StaticVector<NodeHandle, 8> NodeHandles;
		uint32 Count = 0, Target = 0;
		GTSL::StaticVector<ResourceHandle, 8> Children;

		bool isValid() const { return Count >= Target; }
	};
	GTSL::HashMap<uint64, ResourceData, BE::PAR> resources;

	struct DataKeyData {
		uint32 Offset = 0;
		RenderSystem::BufferHandle Buffer;
		GTSL::StaticVector<NodeHandle, 8> Nodes;
		ElementDataHandle Handle;
	};
	GTSL::Vector<DataKeyData, BE::PAR> dataKeys;

	bool getDataKeyState(DataKeyHandle data_key_handle) const { return static_cast<bool>(dataKeys[data_key_handle()].Buffer); }
	RenderSystem::BufferHandle getDataKeyBufferHandle(DataKeyHandle data_key_handle) const { return dataKeys[data_key_handle()].Buffer; }
	uint32 getDataKeyOffset(DataKeyHandle data_key_handle) const { return dataKeys[data_key_handle()].Offset; }

	void onShaderInfosLoaded(TaskInfo taskInfo, ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo shaderInfos, ShaderLoadInfo shaderLoadInfo);

	void onShadersLoaded(TaskInfo taskInfo, ShaderResourceManager*, RenderSystem*, ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo);

	struct DrawData {};

	struct VertexBufferBindData {
		uint32 VertexCount = 0, VertexSize = 0;
		RenderSystem::BufferHandle Handle;
		GTSL::StaticVector<uint32, 8> Offsets;
	};

	struct IndexBufferBindData {
		uint32 IndexCount = 0;
		GAL::IndexType IndexType;		
		RenderSystem::BufferHandle BufferHandle;
	};

	struct IndirectComputeDispatchData {
		
	};

	GTSL::MultiTree<BE::PAR, PublicNode, PipelineBindData, LayerData, RayTraceData, DispatchData, MeshData, RenderPassData, DrawData, VertexBufferBindData, IndexBufferBindData, IndirectComputeDispatchData> renderingTree;

	bool isUnchanged = true;

	NodeHandle addRayTraceNode(const NodeHandle parent_node_handle, const ShaderGroupHandle material_instance_handle) {
		auto handle = addInternalNode<RayTraceData>(222, parent_node_handle);
		getPrivateNode<RayTraceData>(handle).ShaderGroupIndex = material_instance_handle.ShaderGroupIndex;
		return handle;
	}

	NodeHandle addPipelineBindNode(const NodeHandle parent_node_handle, const ShaderGroupHandle material_instance_handle) {
		auto handle = addInternalNode<PipelineBindData>(555, parent_node_handle);
		getPrivateNode<PipelineBindData>(handle).Handle = material_instance_handle;
		BindToNode(handle, shaderGroups[material_instance_handle.ShaderGroupIndex].ResourceHandle);
		return handle;
	}

	auto parseScopeString(const GTSL::StringView parents) const {
		GTSL::StaticVector<GTSL::StaticString<64>, 8> strings;

		{
			uint32 i = 0;

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

	GTSL::StaticMap<Id, NodeHandle, 16> renderPasses;
	GTSL::StaticVector<NodeHandle, 16> renderPassesInOrder;

	GTSL::Extent2D sizeHistory[MAX_CONCURRENT_FRAMES];

	struct Pipeline {
		Pipeline(const BE::PAR& allocator) {}

		GPUPipeline pipeline;
		//ResourceHandle ResourceHandle;
		DataKeyHandle ShaderBindingTableBuffer;

		GTSL::StaticVector<uint64, 16> Shaders;

		struct RayTracingData {
			struct ShaderGroupData {
				MemberHandle TableHandle;

				struct InstanceData {
					MemberHandle ShaderHandle;
					GTSL::StaticVector<MemberHandle, 8> Elements;
				};

				uint32 ShaderCount = 0;

				GTSL::StaticVector<InstanceData, 8> Instances;
			} ShaderGroups[4];

			uint32 PipelineIndex;
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
		uint32 RasterPipelineIndex = 0xFFFFFFFF, ComputePipelineIndex = 0xFFFFFFFF, RTPipelineIndex = 0xFFFFFFFF;
		ResourceHandle ResourceHandle;
	};
	GTSL::FixedVector<ShaderGroupData, BE::PAR> shaderGroups;

	GTSL::HashMap<Id, uint32, BE::PAR> shaderGroupsByName;

	uint32 textureIndex = 0, imageIndex = 0;

	void printNode(const uint32 nodeHandle, uint32 level, bool d) {
		if (!d) { return; }

		switch (renderingTree.GetNodeType(nodeHandle)) {
		case decltype(renderingTree)::GetTypeIndex<LayerData>(): {
			BE_LOG_MESSAGE(u8"Node index: ", nodeHandle, u8", Type: LayerData", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		case decltype(renderingTree)::GetTypeIndex<PipelineBindData>(): {
			BE_LOG_MESSAGE(u8"Node index: ", nodeHandle, u8", Type: Pipeline Bind", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		case decltype(renderingTree)::GetTypeIndex<MeshData>(): {
			BE_LOG_MESSAGE(u8"Node index: ", nodeHandle, u8", Type: Mesh Data", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		case decltype(renderingTree)::GetTypeIndex<RenderPassData>(): {
			BE_LOG_MESSAGE(u8"Node index: ", nodeHandle, u8", Type: Render Pass", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		}
	}

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

	struct TextureLoadInfo {
		TextureLoadInfo() = default;

		TextureLoadInfo(RenderAllocation renderAllocation) : RenderAllocation(renderAllocation)
		{}

		RenderAllocation RenderAllocation;
		RenderSystem::TextureHandle TextureHandle;
	};
	void onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem*, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);
	void onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem*, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);

	struct TextureData {
		ResourceHandle Resource;
		uint32 Index = 0;
	};
	GTSL::HashMap<Id, TextureData, BE::PAR> textures;

	void addPendingResourceToTexture(Id texture, ResourceHandle resource) {
		addDependencyOnResource(resource, textures[texture].Resource);
	}

	struct Attachment {
		RenderSystem::TextureHandle TextureHandle[MAX_CONCURRENT_FRAMES];

		Id Name;
		GAL::TextureUse Uses; GAL::TextureLayout Layout[MAX_CONCURRENT_FRAMES];
		GAL::PipelineStage ConsumingStages; GAL::AccessType AccessType;
		GTSL::RGBA ClearColor; GAL::FormatDescriptor FormatDescriptor;
		uint32 ImageIndex;
	};
	GTSL::HashMap<Id, Attachment, BE::PAR> attachments;

	void updateImage(uint8 frameIndex, Attachment& attachment, GAL::TextureLayout textureLayout, GAL::PipelineStage stages, GAL::AccessType writeAccess) {
		attachment.Layout[frameIndex] = textureLayout; attachment.ConsumingStages = stages; attachment.AccessType = writeAccess;
	}

	DynamicTaskHandle<TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureInfoLoadHandle;
	DynamicTaskHandle<TextureResourceManager::TextureInfo, TextureLoadInfo> onTextureLoadHandle;
	DynamicTaskHandle<ShaderResourceManager::ShaderGroupInfo, ShaderLoadInfo> onShaderInfosLoadHandle;
	DynamicTaskHandle<ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*>, ShaderLoadInfo> onShaderGroupLoadHandle;

	struct ElementData {
		ElementData(const BE::PAR& allocator) : children() {}

		enum class ElementType {
			NULL, SCOPE, TYPE, MEMBER
		} Type = ElementType::NULL;

		GTSL::StaticString<64> DataType, Name;

		struct Member {
			ElementDataHandle TypeHandle;
			uint32 Alignment = 1;
			uint32 Multiplier;
		} Mem;

		struct TypeElement {
			uint32 Size = 0, Alignment = 1;
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
		uint32 multiplier = 1;

		if (auto pos = FindLast(typeString, u8'[')) {
			auto lastBracketPos = FindLast(typeString, u8']');
			multiplier = GTSL::ToNumber<uint32>({ lastBracketPos.Get() - pos.Get() - 1, lastBracketPos.Get() - pos.Get() - 1, typeString.c_str() + pos.Get() + 1 }).Get();
			typeString.Drop(pos.Get());
		}

		if(auto r = tryGetDataTypeHandle(scope, typeString)) {
			typeHandle = r.Get();
		} else {
			return { ElementDataHandle(), false };
		}

		{
			BE_ASSERT(getElement(typeHandle).Type == ElementData::ElementType::TYPE, u8"");

			auto elementResult = tryAddElement(scope, name, ElementData::ElementType::MEMBER);
			auto& element = getElement(elementResult.Get());
			element.Mem.TypeHandle = typeHandle;
			element.Mem.Alignment = getElement(typeHandle).TyEl.Alignment;
			element.Mem.Multiplier = multiplier;

			for (uint32 i = 1, j = parents.GetLength() - 1; i < parents; ++i, --j) {
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

		for (uint32 i = 0; i < scopes.GetLength(); ++i) {
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

	//GTSL::Result<uint32> dataTypeSize(GTSL::StringView parents, GTSL::StringView name) {
	//	auto res = tryGetDataTypeHandle(parents, name);
	//
	//	if (res) {
	//		return { GTSL::MoveRef(elements[res.Get()()].Size), true };
	//	}
	//
	//	return { 0u, false };
	//}

	//will declare data type name under parents
	//2 result if added
	//1 result if exists
	//0 result if failed
	GTSL::Result<ElementDataHandle, uint8> tryAddElement(const GTSL::StringView parents, const GTSL::StringView name, ElementData::ElementType type) {
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
		child.DataType = name;
		child.Type = type;
		return { ElementDataHandle(entry.Get()), 2 };
	}

	ElementData& getElement(const ElementDataHandle element_data_handle) {
		return elements[element_data_handle()];
	}

	const ElementData& getElement(const ElementDataHandle element_data_handle) const {
		return elements[element_data_handle()];
	}

	GTSL::Result<ElementDataHandle> tryAddDataType(const GTSL::StringView parents, const GTSL::StringView name, uint32 size) {
		if (auto r = tryAddElement(parents, name, ElementData::ElementType::TYPE); r.State()) {
			getElement(r.Get()).TyEl.Size = size;
			return { ElementDataHandle(r.Get()), (bool)r.State() };
		} else {
			getElement(r.Get()).TyEl.Size = size;
			return { ElementDataHandle(r.Get()), (bool)r.State() };
		}
	}

#undef NULL

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

	GTSL::Result<GTSL::Pair<ElementDataHandle, uint32>> GetRelativeOffset(const ElementDataHandle element_data_handle, const GTSL::StringView newScope) const {
		ElementDataHandle handle = element_data_handle;

		auto getOffset = [&](const GTSL::StringView scope) -> GTSL::Result<GTSL::Pair<ElementDataHandle, uint32>> {
			uint32 offset = 0;

			if (handle != ElementDataHandle(1)) { //if we are not in global scope
				if (getElement(handle).Type == ElementData::ElementType::MEMBER) {
					handle = getElement(handle).Mem.TypeHandle;
				}

				for (auto& k : elements[handle()].children) {
					auto& t = getElement(k.Handle);

					if(t.Type != ElementData::ElementType::MEMBER) { continue; }

					offset = GTSL::Math::RoundUpByPowerOf2(offset, getElement(t.Mem.TypeHandle).TyEl.Alignment);
					if (k.Name == newScope) { return { { k.Handle, static_cast<uint32&&>(offset) }, true }; }
					offset += getElement(t.Mem.TypeHandle).TyEl.Size;
				}
			}			

			return { { ElementDataHandle(), 0 }, false };
		};

		return getOffset(newScope);
	}

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
				auto& set = sets.EmplaceAt(subSetHandle().SetHandle(), 16, sets.GetAllocator());
				auto& subSet = set.EmplaceAt(subSetHandle().Subset, 32, sets.GetAllocator());
				subSet.EmplaceAt(binding, update);
			}
		}
	};

	GTSL::StaticVector<DescriptorsUpdate, MAX_CONCURRENT_FRAMES> descriptorsUpdates;

	uint32 GetSize(const MemberHandle member_handle) {
		return GetSize(ElementDataHandle(member_handle.Handle));
	}

	uint32 GetSize(const ElementDataHandle element_data_handle, bool getOnlyType = false) {
		auto& e = elements[element_data_handle()];

		switch (e.Type) {
		case ElementData::ElementType::NULL: break;
		case ElementData::ElementType::SCOPE: break;
		case ElementData::ElementType::TYPE: return e.TyEl.Size;
		case ElementData::ElementType::MEMBER: return getElement(e.Mem.TypeHandle).TyEl.Size * (getOnlyType ? 1 : e.Mem.Multiplier);
		}

		BE_ASSERT(false);

		return 0;
	}

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
			GTSL::StaticVector<BindingsPool::BindingsPoolSize, 10> bindingsPoolSizes;

			for (auto& e : bindingDescriptors) {
				bindingsPoolSizes.EmplaceBack(BindingsPool::BindingsPoolSize{ e.BindingType, e.BindingsCount * renderSystem->GetPipelinedFrames() });
				set.SubSets.EmplaceBack(); auto& subSet = set.SubSets.back();
				subSet.Type = e.BindingType;
				subSet.AllocatedBindings = e.BindingsCount;
			}

			for (uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
				set.BindingsPool[f].Initialize(renderSystem->GetRenderDevice(), bindingsPoolSizes, 1);
				set.BindingsSet[f].Initialize(renderSystem->GetRenderDevice(), set.BindingsPool[f], setLayout.BindingsSetLayout);
			}
		}

		return setHandle;
	}

	template<typename T>
	NodeHandle addInternalNode(const uint64 key, NodeHandle publicParentHandle) {
		auto betaNodeHandle = renderingTree.Emplace<T>(0xFFFFFFFF, publicParentHandle());
		isUnchanged = false;
		return NodeHandle(betaNodeHandle);
	}

	friend WorldRendererPipeline;

	byte* buffer[MAX_CONCURRENT_FRAMES]; uint32 offsets[MAX_CONCURRENT_FRAMES]{ 0 };

	struct PendingWriteData {
		void* WhereToRead; void* WhereToWrite = nullptr;
		GTSL::uint64 Size;
		GTSL::Bitfield<MAX_CONCURRENT_FRAMES> FrameCountdown;
	};
	GTSL::HashMap<uint64, PendingWriteData, BE::PAR> pendingWrites;

	GTSL::ShortString<16> tag;

	static constexpr bool INVERSE_Z = true;

#if BE_DEBUG
	GAL::PipelineStage pipelineStages;
#endif
};

class WorldRendererPipeline : public RenderPipeline {
public:
	WorldRendererPipeline(const InitializeInfo& initialize_info);

	auto GetOnAddMeshHandle() const { return OnAddMesh; }
	auto GetOnMeshUpdateHandle() const { return OnUpdateMesh; }

	void onAddShaderGroup(RenderOrchestrator* render_orchestrator, RenderSystem* render_system) {
		++shaderGroupCount;

		if (render_orchestrator->tag == GTSL::ShortString<16>(u8"Visibility")) {
			auto bwk = render_orchestrator->GetBufferWriteKey(render_system, visibilityDataKey);
			bwk[u8"shaderGroupLength"] = shaderGroupCount;
		}
	}

private:
	uint32 shaderGroupCount = 0;

	DynamicTaskHandle<StaticMeshHandle, Id> OnAddMesh;
	DynamicTaskHandle<StaticMeshHandle> OnUpdateMesh;
	DynamicTaskHandle<StaticMeshResourceManager::StaticMeshInfo> onStaticMeshLoadHandle;
	DynamicTaskHandle<StaticMeshResourceManager::StaticMeshInfo> onStaticMeshInfoLoadHandle;

	DynamicTaskHandle<StaticMeshHandle, Id, ShaderGroupHandle> OnAddInfiniteLight;

	DynamicTaskHandle<StaticMeshHandle, Id, ShaderGroupHandle> OnAddBackdrop;
	DynamicTaskHandle<StaticMeshHandle, Id, ShaderGroupHandle> OnAddParticleSystem;
	DynamicTaskHandle<StaticMeshHandle, Id, ShaderGroupHandle> OnAddVolume;
	DynamicTaskHandle<StaticMeshHandle, Id, ShaderGroupHandle> OnAddSkinnedMesh;

	RenderOrchestrator::MemberHandle staticMeshStruct;
	RenderOrchestrator::MemberHandle matrixUniformBufferMemberHandle;
	RenderOrchestrator::MemberHandle vertexBufferReferenceHandle, indexBufferReferenceHandle;
	RenderOrchestrator::NodeHandle staticMeshRenderGroup;
	RenderOrchestrator::MemberHandle staticMeshInstanceDataStruct;

	GTSL::MultiVector<BE::PAR, false, float32, float32, float32, float32> spherePositionsAndRadius;

	GTSL::StaticVector<GTSL::Pair<Id, StaticMeshHandle>, 8> pendingAdditions;
	GTSL::StaticVector<RenderSystem::AccelerationStructureHandle, 8> pendingBuilds;

	bool rayTracing = false;
	RenderSystem::AccelerationStructureHandle topLevelAccelerationStructure;
	RenderOrchestrator::NodeHandle vertexBufferNodeHandle, indexBufferNodeHandle, meshDataNode;
	RenderOrchestrator::NodeHandle mainVisibilityPipelineNode;
	Handle<uint32, DataKey_tag> visibilityDataKey;

	struct Mesh {
		ShaderGroupHandle MaterialHandle;
		RenderSystem::BLASInstanceHandle InstanceHandle;
	};
	GTSL::HashMap<StaticMeshHandle, Mesh, BE::PAR> meshes;

	RenderOrchestrator::DataKeyHandle meshDataBuffer;

	struct Resource {
		GTSL::StaticVector<GTSL::StaticVector<GAL::ShaderDataType, 8>, 8> VertexElements;
		GTSL::StaticVector<StaticMeshHandle, 8> Meshes;
		bool Loaded = false;
		uint32 Offset = 0, IndexOffset = 0;
		uint32 VertexSize, VertexCount = 0, IndexCount = 0;
		GAL::IndexType IndexType;
		RenderSystem::AccelerationStructureHandle BLAS;
		GTSL::Vector3 ScalingFactor = GTSL::Vector3(1.0f);
		bool Interleaved = true;
		RenderOrchestrator::NodeHandle nodeHandle;
	};
	GTSL::HashMap<Id, Resource, BE::PAR> resources;

	RenderSystem::BufferHandle vertexBuffer, indexBuffer;
	uint32 vertexBufferOffset = 0, indexBufferOffset = 0;

	struct MaterialData {
		RenderOrchestrator::NodeHandle Node;
		ShaderGroupHandle SGHandle;
	};
	GTSL::HashMap<uint32, MaterialData, BE::PAR> materials;

	RenderOrchestrator::NodeHandle visibilityRenderPassNodeHandle, lightingDataNodeHandle;

	static uint32 calculateMeshSize(const uint32 vertexCount, const uint32 vertexSize, const uint32 indexCount, const uint32 indexSize) {
		return GTSL::Math::RoundUpByPowerOf2(vertexCount * vertexSize, 16) + indexCount * indexSize;
	}

	void onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo) {
		auto& resource = resources[Id(staticMeshInfo.GetName())];

		auto verticesSize = staticMeshInfo.GetVertexSize() * staticMeshInfo.GetVertexCount(), indicesSize = staticMeshInfo.GetIndexCount() * staticMeshInfo.GetIndexSize();

		resource.VertexSize = staticMeshInfo.GetVertexSize();
		resource.VertexCount = staticMeshInfo.VertexCount;
		resource.IndexCount = staticMeshInfo.IndexCount;
		resource.IndexType = GAL::SizeToIndexType(staticMeshInfo.IndexSize);
		resource.Interleaved = staticMeshInfo.Interleaved;

		resource.Offset = vertexBufferOffset; resource.IndexOffset = indexBufferOffset;

		for(uint32 i = 0; i < staticMeshInfo.GetSubMeshes().Length; ++i) {
			auto& sm = staticMeshInfo.GetSubMeshes().array[i];
			auto shaderGroupHandle = render_orchestrator->CreateShaderGroup(Id(sm.ShaderGroupName));

			if (render_orchestrator->tag == GTSL::ShortString<16>(u8"Forward")) {
				if (auto r = materials.TryEmplace(shaderGroupHandle.ShaderGroupIndex)) {
					auto materialDataNode = render_orchestrator->AddDataNode(meshDataNode, u8"MaterialNode", render_orchestrator->shaderGroups[shaderGroupHandle.ShaderGroupIndex].Buffer);
					r.Get().Node = render_orchestrator->AddMaterial(render_system, materialDataNode, shaderGroupHandle);
					
					resource.nodeHandle = render_orchestrator->AddMesh(r.Get().Node, 0, resource.IndexCount, vertexBufferOffset);
				}
			}
			else if (render_orchestrator->tag == GTSL::ShortString<16>(u8"Visibility")) {
				if (auto r = materials.TryEmplace(shaderGroupHandle.ShaderGroupIndex)) {
					resource.nodeHandle = render_orchestrator->AddMesh(mainVisibilityPipelineNode, 0, resource.IndexCount, vertexBufferOffset);
				}

				//TODO: add to selection buffer
				//TODO: add pipeline bind to render pixels with this material

				//render_orchestrator->AddIndirectDispatchNode();
			}
		}

		//if unorm or snorm is used to specify data, take that into account as some properties (such as positions) may need scaling as XNORM enconding is defined in the 0->1 / -1->1 range
		bool usesxNorm = false;

		for (uint32 ai = 0; ai < staticMeshInfo.GetVertexDescriptor().Length; ++ai) {
			auto& t = resource.VertexElements.EmplaceBack();

			auto& a = staticMeshInfo.GetVertexDescriptor().array[ai];
			for (uint32 bi = 0; bi < a.Length; ++bi) {
				auto& b = a.array[bi];

				t.EmplaceBack(b);

				if (b == GAL::ShaderDataType::U16_UNORM or b == GAL::ShaderDataType::U16_UNORM2 or b == GAL::ShaderDataType::U16_UNORM3 or b == GAL::ShaderDataType::U16_UNORM4) {
					usesxNorm = true;
				}

				if (b == GAL::ShaderDataType::U16_SNORM or b == GAL::ShaderDataType::U16_SNORM2 or b == GAL::ShaderDataType::U16_SNORM3 or b == GAL::ShaderDataType::U16_SNORM4) {
					usesxNorm = true;
				}				
			}

		}

		if(usesxNorm) {
			//don't always assign bounding box as scaling factor, as even if we didn't need it bounding boxes usually have little errors which would cause the mesh to be scaled incorrectly
			//even though we have the correct coordinates to begin with
			resource.ScalingFactor = staticMeshInfo.GetBoundingBox();
		}

		staticMeshResourceManager->LoadStaticMesh(taskInfo.ApplicationManager, staticMeshInfo, vertexBufferOffset, render_system->GetBufferRange(vertexBuffer), indexBufferOffset, render_system->GetBufferRange(indexBuffer), onStaticMeshLoadHandle);

		vertexBufferOffset += staticMeshInfo.GetVertexCount();
		indexBufferOffset += staticMeshInfo.GetIndexCount();
	}

	void onStaticMeshLoaded(TaskInfo taskInfo, RenderSystem* render_system, StaticMeshRenderGroup* render_group, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo) {
		auto& res = resources[Id(staticMeshInfo.GetName())];

		auto commandListHandle = render_orchestrator->buildCommandList[render_system->GetCurrentFrame()];

		render_system->UpdateBuffer(commandListHandle, vertexBuffer); render_system->UpdateBuffer(commandListHandle, indexBuffer);
		render_orchestrator->AddVertices(vertexBufferNodeHandle, staticMeshInfo.GetVertexCount());
		render_orchestrator->AddIndices(indexBufferNodeHandle, staticMeshInfo.GetIndexCount());

		if (rayTracing) {
			res.BLAS = render_system->CreateBottomLevelAccelerationStructure(staticMeshInfo.VertexCount, 12/*todo: use actual position stride*/, staticMeshInfo.IndexCount, GAL::SizeToIndexType(staticMeshInfo.IndexSize), vertexBuffer, indexBuffer, res.Offset * 12/*todo: use actual position coordinate element size*/, res.IndexOffset);
			pendingBuilds.EmplaceBack(res.BLAS);
		}

		for (const auto e : res.Meshes) {
			onMeshLoad(render_system, render_group, render_orchestrator, res, Id(staticMeshInfo.GetName()), e);
			*spherePositionsAndRadius.GetPointer<3>(e()) = staticMeshInfo.BoundingRadius;
		}

		res.Loaded = true;

		GTSL::StaticVector<GTSL::Range<const GAL::ShaderDataType*>, 8> r;

		for (auto& e : res.VertexElements) {
			r.EmplaceBack(e.GetRange());
		}
	}

	//BUG: WE HAVE AN IMPLICIT DEPENDENCY ON ORDERING OF TASK, AS WE REQUIRE onAddMesh TO BE RUN BEFORE updateMesh, THIS ORDERING IS NOT CURRENTLY GUARANTEED BY THE TASK SYSTEM

	void onAddMesh(TaskInfo task_info, StaticMeshResourceManager* static_mesh_resource_manager, RenderOrchestrator* render_orchestrator, RenderSystem* render_system, StaticMeshRenderGroup* static_mesh_render_group, StaticMeshHandle static_mesh_handle, Id resourceName) {
		auto& mesh = meshes.Emplace(static_mesh_handle);
		auto resource = resources.TryEmplace(resourceName);

		resource.Get().Meshes.EmplaceBack(static_mesh_handle);
		spherePositionsAndRadius.EmplaceBack(0, 0, 0, 0);

		if (rayTracing) {
			mesh.InstanceHandle = render_system->AddBLASToTLAS(topLevelAccelerationStructure, resource.Get().BLAS, static_mesh_handle(), mesh.InstanceHandle);
		}

		if (resource) {
			static_mesh_resource_manager->LoadStaticMeshInfo(task_info.ApplicationManager, resourceName, onStaticMeshInfoLoadHandle);
		}
		else {
			if (resource.Get().Loaded) {
				onMeshLoad(render_system, static_mesh_render_group, render_orchestrator, resource.Get(), resourceName, static_mesh_handle);
			}
		}
	}

	void onMeshLoad(RenderSystem* renderSystem, StaticMeshRenderGroup* renderGroup, RenderOrchestrator* render_orchestrator, const Resource& res, Id resource_name, StaticMeshHandle static_mesh_handle) {
		auto& mesh = meshes[static_mesh_handle];

		auto key = render_orchestrator->GetBufferWriteKey(renderSystem, meshDataBuffer);
		key[static_mesh_handle()][u8"transform"] = GTSL::Matrix3x4(renderGroup->GetMeshTransform(static_mesh_handle));
		key[static_mesh_handle()][u8"vertexBufferOffset"] = res.Offset; key[static_mesh_handle()][u8"indexBufferOffset"] = res.IndexOffset;
		key[static_mesh_handle()][u8"shaderGroupIndex"] = mesh.MaterialHandle.ShaderGroupIndex; //TODO: maybe use ACTUAL pipeline index to take into account instances

		render_orchestrator->AddInstance(res.nodeHandle);

		if (rayTracing) {
			pendingAdditions.EmplaceBack(resource_name, static_mesh_handle);
		}
	}

	void updateMesh(TaskInfo, RenderSystem* renderSystem, StaticMeshRenderGroup* renderGroup, RenderOrchestrator* render_orchestrator, StaticMeshHandle static_mesh_handle) {
		auto key = render_orchestrator->GetBufferWriteKey(renderSystem, meshDataBuffer);
		auto pos = renderGroup->GetMeshTransform(static_mesh_handle);


		//info.MaterialSystem->UpdateIteratorMember(bufferIterator, staticMeshStruct, renderGroup->GetMeshIndex(e));
		key[static_mesh_handle()][u8"transform"] = pos;
		*spherePositionsAndRadius.GetPointer<0>(static_mesh_handle()) = pos[0][3];
		*spherePositionsAndRadius.GetPointer<1>(static_mesh_handle()) = pos[1][3];
		*spherePositionsAndRadius.GetPointer<2>(static_mesh_handle()) = pos[2][3];

		if (rayTracing) {
			renderSystem->SetInstancePosition(topLevelAccelerationStructure, meshes[static_mesh_handle].InstanceHandle, pos);
		}
	}

	uint32 lights = 0;

	void onAddLight(TaskInfo, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, LightsRenderGroup::PointLightHandle light_handle) {
		auto bwk = render_orchestrator->GetBufferWriteKey(render_system, lightingDataNodeHandle);
		bwk[u8"pointLightsLength"] = ++lights;
		bwk[u8"pointLights"][light_handle()][u8"position"] = GTSL::Vector3(0, 0, 0);
		bwk[u8"pointLights"][light_handle()][u8"color"] = GTSL::Vector3(1, 1, 1);
		bwk[u8"pointLights"][light_handle()][u8"intensity"] = 5.f;
	}

	void updateLight(const TaskInfo, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, LightsRenderGroup::PointLightHandle light_handle, GTSL::Vector3 position, GTSL::RGB color, float32 intensity) {
		auto bwk = render_orchestrator->GetBufferWriteKey(render_system, lightingDataNodeHandle);
		bwk[u8"pointLights"][light_handle()][u8"position"] = position;
		bwk[u8"pointLights"][light_handle()][u8"color"] = color;
		bwk[u8"pointLights"][light_handle()][u8"intensity"] = intensity;
	}

	void preRender(TaskInfo, RenderSystem* render_system, RenderOrchestrator* render_orchestrator) {
		//GTSL::Vector<float32, BE::TAR> results(GetTransientAllocator());
		//projectSpheres({0}, spherePositionsAndRadius, results);

		{ // Add BLAS instances to TLAS only if dependencies were fulfilled
			auto i = 0;

			while (i < pendingAdditions) {
				const auto& addition = pendingAdditions[i];
				auto e = addition.Second;
				auto& mesh = meshes[e];

				mesh.InstanceHandle = render_system->AddBLASToTLAS(topLevelAccelerationStructure, resources[addition.First].BLAS, e(), mesh.InstanceHandle);

				pendingAdditions.Pop(i);
				++i;
			}
		}


		auto workloadHandle = render_orchestrator->buildAccelerationStructuresWorkloadHandle[render_system->GetCurrentFrame()];
		render_system->Wait(workloadHandle);
		render_system->StartCommandList(render_orchestrator->buildCommandList[render_system->GetCurrentFrame()]);

		if (rayTracing) {
			render_system->DispatchBuild(render_orchestrator->buildCommandList[render_system->GetCurrentFrame()], pendingBuilds); //Update all BLASes
			pendingBuilds.Resize(0);
			render_system->DispatchBuild(render_orchestrator->buildCommandList[render_system->GetCurrentFrame()], { topLevelAccelerationStructure }); //Update TLAS
		}

		render_system->EndCommandList(render_orchestrator->buildCommandList[render_system->GetCurrentFrame()]);
		render_system->Submit(GAL::QueueTypes::COMPUTE, { { { render_orchestrator->buildCommandList[render_system->GetCurrentFrame()] }, {  }, { workloadHandle } } }, workloadHandle);
	}

	void terrain() {
		struct TerrainVertex {
			GTSL::Vector3 position; GTSL::RGBA color;
		};

		GTSL::Extent3D terrainExtent{ 256, 1, 256 };

		uint32 vertexCount = (terrainExtent.Width - 1) * (terrainExtent.Depth - 1) * 8;
		uint32 indexCount = vertexCount;

		TerrainVertex* vertices = nullptr; uint16* indices = nullptr;

		// Initialize the index into the vertex and index arrays.
		uint32 index = 0;

		GTSL::RGBA color; uint32 m_terrainWidth; GTSL::Vector3* m_terrainModel = nullptr, * m_heightMap = nullptr;

		// Load the vertex array and index array with data.
		for (uint32 j = 0; j < (terrainExtent.Depth - 1); j++) {
			for (uint32 i = 0; i < (terrainExtent.Width - 1); i++) {
				// Get the indexes to the four points of the quad.
				uint32 index1 = (m_terrainWidth * j) + i;          // Upper left.
				uint32 index2 = (m_terrainWidth * j) + (i + 1);      // Upper right.
				uint32 index3 = (m_terrainWidth * (j + 1)) + i;      // Bottom left.
				uint32 index4 = (m_terrainWidth * (j + 1)) + (i + 1);  // Bottom right.

				// Now create two triangles for that quad.
				// Triangle 1 - Upper left.
				m_terrainModel[index].X() = m_heightMap[index1].X();
				m_terrainModel[index].Y() = m_heightMap[index1].Y();
				m_terrainModel[index].Z() = m_heightMap[index1].Z();
				index++;

				// Triangle 1 - Upper right.
				m_terrainModel[index].X() = m_heightMap[index2].X();
				m_terrainModel[index].Y() = m_heightMap[index2].Y();
				m_terrainModel[index].Z() = m_heightMap[index2].Z();
				index++;

				// Triangle 1 - Bottom left.
				m_terrainModel[index].X() = m_heightMap[index3].X();
				m_terrainModel[index].Y() = m_heightMap[index3].Y();
				m_terrainModel[index].Z() = m_heightMap[index3].Z();
				index++;

				// Triangle 2 - Bottom left.
				m_terrainModel[index].X() = m_heightMap[index3].X();
				m_terrainModel[index].Y() = m_heightMap[index3].Y();
				m_terrainModel[index].Z() = m_heightMap[index3].Z();
				index++;

				// Triangle 2 - Upper right.
				m_terrainModel[index].X() = m_heightMap[index2].X();
				m_terrainModel[index].Y() = m_heightMap[index2].Y();
				m_terrainModel[index].Z() = m_heightMap[index2].Z();
				index++;

				// Triangle 2 - Bottom right.
				m_terrainModel[index].X() = m_heightMap[index4].X();
				m_terrainModel[index].Y() = m_heightMap[index4].Y();
				m_terrainModel[index].Z() = m_heightMap[index4].Z();
				index++;
			}
		}
	}

	void setupDirectionShadowRenderPass(RenderSystem* renderSystem, RenderOrchestrator* renderOrchestrator) {
		// Make render pass
		RenderOrchestrator::PassData pass_data;
		pass_data.PassType = RenderOrchestrator::PassType::RAY_TRACING;
		pass_data.Attachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ GAL::AccessTypes::WRITE, u8"Color" });
		pass_data.Attachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ GAL::AccessTypes::READ, u8"WorldPosition" });
		pass_data.Attachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ GAL::AccessTypes::READ, u8"RenderDepth" });
		auto renderPassLayerHandle = renderOrchestrator->AddRenderPass(u8"DirectionalShadow", renderOrchestrator->GetGlobalDataLayer(), renderSystem, pass_data, GetApplicationManager());

		// Create shader group
		auto rayTraceShaderGroupHandle = renderOrchestrator->CreateShaderGroup(u8"DirectionalShadow");
		// Add dispatch
		auto pipelineBindNode = renderOrchestrator->addPipelineBindNode(renderPassLayerHandle, rayTraceShaderGroupHandle);
		auto cameraDataNode = renderOrchestrator->AddDataNode(pipelineBindNode, u8"CameraData", renderOrchestrator->cameraDataKeyHandle);
		
		auto traceRayParameterDataHandle = renderOrchestrator->CreateMember2(u8"global", u8"TraceRayParameterData", { { u8"uint64", u8"accelerationStructure" }, { u8"uint32", u8"rayFlags" }, { u8"uint32", u8"recordOffset"}, { u8"uint32", u8"recordStride"}, { u8"uint32", u8"missIndex"}, { u8"float32", u8"tMin"}, { u8"float32", u8"tMax"} });
		auto rayTraceDataMember = renderOrchestrator->CreateMember2(u8"global", u8"RayTraceData", { { u8"TraceRayParameterData", u8"traceRayParameters" }, { u8"StaticMeshData*", u8"staticMeshes" } });
		auto rayTraceDataNode = renderOrchestrator->AddDataNode(u8"RayTraceData", cameraDataNode, rayTraceDataMember);

		auto rayTraceNode = renderOrchestrator->addRayTraceNode(rayTraceDataNode, rayTraceShaderGroupHandle);

		auto bwk = renderOrchestrator->GetBufferWriteKey(renderSystem, rayTraceDataNode);
		bwk[u8"traceRayParameters"][u8"accelerationStructure"] = topLevelAccelerationStructure;
		bwk[u8"traceRayParameters"][u8"rayFlags"] = 0u;
		bwk[u8"traceRayParameters"][u8"recordOffset"] = 0u;
		bwk[u8"traceRayParameters"][u8"recordStride"] = 0u;
		bwk[u8"traceRayParameters"][u8"missIndex"] = 0u;
		bwk[u8"traceRayParameters"][u8"tMin"] = 0.001f;
		bwk[u8"traceRayParameters"][u8"tMax"] = 100.0f;
		bwk[u8"staticMeshes"] = meshDataBuffer;
	}
};

class UIRenderManager : public RenderManager {
public:
	UIRenderManager(const InitializeInfo& initializeInfo) : RenderManager(initializeInfo, u8"UIRenderManager") {
		auto* renderSystem = initializeInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");
		auto* renderOrchestrator = initializeInfo.ApplicationManager->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");

		renderOrchestrator->CreateMember2(u8"global", u8"Segment", { { u8"vec2f[3]", u8"points" } });
		renderOrchestrator->CreateMember2(u8"global", u8"FontRenderData", { { u8"uint32[256]", u8"offsets" }, { u8"Segment[256 * 2048]", u8"segments"} });

		renderOrchestrator->CreateMember2(u8"global", u8"TextRenderData", { { u8"FontRenderData[4]", u8"fonts" } });

		renderOrchestrator->CreateMember2(u8"global", u8"TextBlockData", { { u8"uint32", u8"fontIndex" }, { u8"uint32[256]", u8"chars"} });

		//RenderOrchestrator::CreateMaterialInfo createMaterialInfo;
		//createMaterialInfo.RenderSystem = renderSystem;
		//createMaterialInfo.ApplicationManager = initializeInfo.ApplicationManager;
		//createMaterialInfo.MaterialName = "UIMat";
		//createMaterialInfo.InstanceName = "UIMat";
		//createMaterialInfo.ShaderResourceManager = BE::Application::Get()->GetResourceManager<ShaderResourceManager>("ShaderResourceManager");
		//createMaterialInfo.TextureResourceManager = BE::Application::Get()->GetResourceManager<TextureResourceManager>("TextureResourceManager");
		//uiMaterial = renderOrchestrator->CreateShaderGroup(createMaterialInfo);
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

		GTSL::Matrix3x4 matrix; //default identity matrix

		//UIManager* ui;
		//
		//ui->ProcessUpdates();
		//
		//auto& canvases = ui->GetCanvases();
		//for(auto& e : canvases) {
		//	auto& primitive = ui->getPrimitive(e);
		//}
	}

	void ui() {
		UIManager* ui;
		UIManager::TextPrimitive textPrimitive{ GetPersistentAllocator() };

		RenderOrchestrator::BufferWriteKey text;
		text[u8"fontIndex"] = fontOrderMap[Id(textPrimitive.Font)];

		for (uint32 i = 0; i < textPrimitive.Text; ++i) {
			text[u8"chars"][i] = fontCharMap[textPrimitive.Text[i]];
		}

		for(const auto& e : ui->GetCanvases()) {
		}

		//ui->GetText
	}

	ShaderGroupHandle GetUIMaterial() const { return uiMaterial; }

private:
	RenderOrchestrator::MemberHandle matrixUniformBufferMemberHandle, colorHandle;
	RenderOrchestrator::MemberHandle uiDataStruct;

	GTSL::StaticMap<Id, uint32, 4> fontOrderMap;
	GTSL::StaticMap<char32_t, uint32, 4> fontCharMap;

	uint8 comps = 2;
	ShaderGroupHandle uiMaterial;
};
