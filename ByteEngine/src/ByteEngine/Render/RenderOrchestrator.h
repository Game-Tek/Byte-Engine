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
		GTSL::Matrix4 ViewMatrix, ProjectionMatrix;
	};
	virtual void Setup(const SetupInfo& info) = 0;


	
protected:
	GTSL::Vector<MaterialHandle, BE::PAR> materials;
};

class StaticMeshRenderManager : public RenderManager
{
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;

private:
	MemberHandle matrixUniformBufferMemberHandle;
	uint64 staticMeshDataStructHandle;

	SetHandle dataSet;
};

class UIRenderManager : public RenderManager
{
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;

private:
	RenderSystem::GPUMeshHandle square;

	MemberHandle matrixUniformBufferMemberHandle;
	uint64 uiDataStructHandle;

	SetHandle dataSet;

	uint8 comps = 0, comps2 = 1;
	MaterialHandle uiMaterial;
};

class RenderOrchestrator : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Setup(TaskInfo taskInfo);
	void Render(TaskInfo taskInfo);

	void AddRenderManager(GameInstance* gameInstance, const Id renderManager, const uint16 systemReference);
	void RemoveRenderManager(GameInstance* gameInstance, const Id renderManager, const uint16 systemReference);

	void AddAttachment(RenderSystem* renderSystem, Id name, TextureFormat format, TextureUses::value_type uses, TextureType::value_type type);
	GTSL::Range<const GTSL::RGBA*> GetClearValues(const uint8 rp)
	{
		auto& renderPass = apiRenderPasses[rp];
		return renderPass.ClearValues;
	}

	struct AttachmentInfo
	{
		Id Name;
		TextureLayout StartState, EndState;
		GAL::RenderTargetLoadOperations Load;
		GAL::RenderTargetStoreOperations Store;
	};

	struct PassData
	{
		GTSL::Array<Id, 8> ReadAttachments, WriteAttachments;
		GTSL::Array<TextureLayout, 8> ReadAttachmentsLayouts, WriteAttachmentsLayouts;
		Id Name;

		struct AttachmentUse
		{
			Id Name;
			TextureLayout Layout;
		};
		AttachmentUse DepthStencilAttachment;
	};
	void AddPass(RenderSystem* renderSystem, GTSL::Range<const AttachmentInfo*> attachmentInfos, GTSL::Range<const PassData*> passesData);

	void OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize);

	[[nodiscard]] RenderPass getAPIRenderPass(const uint8 rp) const { return apiRenderPasses[rp].RenderPass; }

	uint8 GetRenderPassIndex(const Id name) const { return apiRenderPassesMap.At(name); }
	[[nodiscard]] uint8 GetSubPassIndex(const uint8 renderPass, const Id subPassName) const { return subPassMap[renderPass].At(subPassName); }

	[[nodiscard]] FrameBuffer getFrameBuffer(const uint8 rp) const { return apiRenderPasses[rp].FrameBuffer; }
	[[nodiscard]] uint8 getAPIRenderPassesCount() const { return apiRenderPasses.GetLength(); }
	[[nodiscard]] uint8 GetSubPassCount(const uint8 renderPass) const { return subPasses[renderPass].GetLength(); }

	Id GetRenderPassName(const uint8 rp) { return apiRenderPasses[rp].Name; }
	Id GetSubPassName(const uint8 rp, const uint8 sp) { return subPasses[rp][sp].Name; }
	
	void AddToRenderPass(Id renderPass, Id renderGroup)
	{
		if(renderPassesMap.Find(renderPass))
		{
			renderPassesMap.At(renderPass).RenderGroups.EmplaceBack(renderGroup);
		}
	}
private:
	inline static const Id RENDER_TASK_NAME{ "RenderRenderGroups" };
	inline static const Id SETUP_TASK_NAME{ "SetupRenderGroups" };
	inline static const Id CLASS_NAME{ "RenderOrchestrator" };
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 32>, BE::PersistentAllocatorReference> setupSystemsAccesses;
	
	GTSL::FlatHashMap<uint16, BE::PersistentAllocatorReference> renderManagers;


	struct RenderPassData
	{
		GTSL::Array<Id, 8> RenderGroups;
	};
	GTSL::FlatHashMap<RenderPassData, BE::PAR> renderPassesMap;
	GTSL::Array<Id, 8> renderPassesNames;

	using RenderPassFunctionType = GTSL::FunctionPointer<void(RenderSystem*, MaterialSystem*, uint32[4], CommandBuffer, PipelineLayout, uint8)>;
	
	GTSL::Array<RenderPassFunctionType, 8> renderPassesFunctions;

	void renderScene(RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 pushConstant[4], CommandBuffer commandBuffer, PipelineLayout pipelineLayout, uint8 rp);
	void renderUI(RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 pushConstant[4], CommandBuffer commandBuffer, PipelineLayout pipelineLayout, uint8 rp);

	struct RenderPassAttachment
	{
		TextureLayout Layout;
		uint8 Index;
	};

	struct APIRenderPassData
	{
		Id Name;
		RenderPass RenderPass;
		GTSL::StaticMap<RenderPassAttachment, 8> Attachments;
		GTSL::Array<GTSL::RGBA, 8> ClearValues;
		GTSL::Array<Id, 8> AttachmentNames;

		/**
		 * \brief Handles to application defined render passes that have to occur in this GAL render api.
		 */
		GTSL::Array<uint8, 8> RenderPasses;
		
		FrameBuffer FrameBuffer;
	};
	GTSL::Array<APIRenderPassData, 16> apiRenderPasses;
	GTSL::StaticMap<uint8, 16> apiRenderPassesMap;

	struct SubPass
	{
		Id Name;
		uint8 DepthAttachment;
	};
	GTSL::Array<GTSL::Array<SubPass, 16>, 16> subPasses;

	GTSL::Array<GTSL::StaticMap<uint8, 16>, 8> subPassMap;

	struct Attachment
	{
		TextureFormat Format;
		Texture Texture;
		TextureView TextureView;
		TextureSampler TextureSampler;

		GTSL::RGBA ClearValue;

		RenderAllocation Allocation;

		Id Name;
		TextureType::value_type Type;
		TextureUses::value_type Uses;
	};
	GTSL::StaticMap<Attachment, 32> attachments;

public:
	Texture GetAttachmentTexture(const Id attachment) const { return attachments.At(attachment).Texture; }
	TextureView GetAttachmentTextureView(const Id attachment) const { return attachments.At(attachment).TextureView; }
	TextureSampler GetAttachmentTextureSampler(const Id attachment) const { return attachments.At(attachment).TextureSampler; }
};
