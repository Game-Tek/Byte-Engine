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

	SetHandle dataSet;
};

class UIRenderManager : public RenderManager
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies) override;

	void Setup(const SetupInfo& info) override;
	RenderSystem::GPUMeshHandle GetSquareMesh() const { return square; }
	MaterialHandle GetUIMaterial() const { return uiMaterial; }

private:
	RenderSystem::GPUMeshHandle square;

	MemberHandle matrixUniformBufferMemberHandle, colorHandle;

	SetHandle dataSet;

	uint8 comps = 2;
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
		
		struct AttachmentReference
		{
			Id Name;
		};
		GTSL::Array<AttachmentReference, 8> ReadAttachments, WriteAttachments;

		PassType PassType;

		AttachmentReference DepthStencilAttachment;
	};
	void AddPass(RenderSystem* renderSystem, GTSL::Range<const PassData*> passesData);

	void OnResize(RenderSystem* renderSystem, MaterialSystem* materialSystem, const GTSL::Extent2D newSize);

	[[nodiscard]] RenderPass getAPIRenderPass(const Id renderPassName) const { return apiRenderPasses[renderPassesMap.At(renderPassName()).APIRenderPass].RenderPass; }

	uint8 GetRenderPassIndex(const Id name) const { return renderPassesMap.At(name()).APIRenderPass; }
	[[nodiscard]] uint8 getAPISubPassIndex(const Id renderPass) const
	{
		uint8 i = 0;
		
		for(auto& e : subPasses[renderPassesMap.At(renderPass()).APIRenderPass]) { if (e.Name == renderPass) { return i; } } 
	}

	[[nodiscard]] FrameBuffer getFrameBuffer(const uint8 rp) const { return apiRenderPasses[rp].FrameBuffer; }
	[[nodiscard]] uint8 getAPIRenderPassesCount() const { return apiRenderPasses.GetLength(); }
	[[nodiscard]] uint8 GetSubPassCount(const uint8 renderPass) const { return subPasses[renderPass].GetLength(); }

	Id GetSubPassName(const uint8 rp, const uint8 sp) { return subPasses[rp][sp].Name; }

	//Enables or disables render passes
	//Right now we have to guarantee enabledRenderPasses has the correct order
	//TODO: maybe everytime we have to execute a render pass check if is enabled so we don't have to keep order between the two collections
	void ToggleRenderPass(Id renderPassName, bool enable)
	{
		renderPassesMap[renderPassName()].Enabled = enable;
	}
	
	void AddToRenderPass(Id renderPass, Id renderGroup)
	{
		if(renderPassesMap.Find(renderPass()))
		{
			renderPassesMap.At(renderPass()).RenderGroups.EmplaceBack(renderGroup);
		}
	}
private:
	inline static const Id RENDER_TASK_NAME{ "RenderRenderGroups" };
	inline static const Id SETUP_TASK_NAME{ "SetupRenderGroups" };
	inline static const Id CLASS_NAME{ "RenderOrchestrator" };
	
	GTSL::Vector<Id, BE::PersistentAllocatorReference> systems;
	GTSL::Vector<GTSL::Array<TaskDependency, 32>, BE::PersistentAllocatorReference> setupSystemsAccesses;
	
	GTSL::FlatHashMap<uint16, BE::PersistentAllocatorReference> renderManagers;

	GTSL::Array<Id, 8> renderPasses;

	struct AttachmentData
	{
		Id Name;
		TextureLayout Layout;
		AccessFlags AccessFlags;
	};
	
	struct RenderPassData
	{
		GTSL::Array<Id, 8> RenderGroups;
		PassType PassType;

		GTSL::Array<AttachmentData, 8> WriteAttachments, ReadAttachments;

		bool Enabled = false;
		
		uint8 APIRenderPass = 0;
		PipelineStage::value_type PipelineStages;
	};
	GTSL::FlatHashMap<RenderPassData, BE::PAR> renderPassesMap;
	GTSL::Array<Id, 8> renderPassesNames;

	using RenderPassFunctionType = GTSL::FunctionPointer<void(GameInstance*, RenderSystem*, MaterialSystem*, CommandBuffer, Id)>;
	
	GTSL::StaticMap<RenderPassFunctionType, 8> renderPassesFunctions;

	void renderScene(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);
	void renderUI(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);
	void renderRays(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);

	void transitionImages(CommandBuffer commandBuffer, RenderSystem* renderSystem, Id renderPassId);

	struct APIRenderPassData
	{
		RenderPass RenderPass;
		
		FrameBuffer FrameBuffer;
		GTSL::Array<Id, 8> AttachmentNames;
	};
	GTSL::Array<APIRenderPassData, 16> apiRenderPasses;

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

		RenderAllocation Allocation;

		Id Name;
		TextureType::value_type Type;
		TextureUses::value_type Uses;
		TextureLayout Layout;
		AccessFlags::value_type AccessFlags;
	};
	GTSL::StaticMap<Attachment, 32> attachments;
	
	void updateImage(Attachment& attachment, AccessFlags::value_type accessFlags, TextureLayout textureLayout)
	{
		attachment.AccessFlags = accessFlags; attachment.Layout = textureLayout;
	}

public:
	Texture GetAttachmentTexture(const Id attachment) const { return attachments.At(attachment()).Texture; }
	TextureView GetAttachmentTextureView(const Id attachment) const { return attachments.At(attachment()).TextureView; }
	TextureSampler GetAttachmentTextureSampler(const Id attachment) const { return attachments.At(attachment()).TextureSampler; }
};
