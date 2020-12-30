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
	uint64 uiDataStructHandle;

	SetHandle dataSet;

	uint8 comps = 0, comps2 = 2;
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
	void AddPass(RenderSystem* renderSystem, GTSL::Range<const AttachmentInfo*> attachmentInfos, GTSL::Range<const PassData*> passesData);

	void OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize);

	[[nodiscard]] RenderPass getAPIRenderPass(const Id renderPassName) const { return apiRenderPasses[renderPassesMap.At(renderPassName()).APIRenderPass].RenderPass; }

	uint8 GetRenderPassIndex(const Id name) const { return renderPassesMap.At(name()).APIRenderPass; }
	[[nodiscard]] uint8 getAPISubPassIndex(const Id renderPass) const { return renderPassesMap.At(renderPass()).APISubPass; }

	[[nodiscard]] FrameBuffer getFrameBuffer(const uint8 rp) const { return apiRenderPasses[rp].FrameBuffer; }
	[[nodiscard]] uint8 getAPIRenderPassesCount() const { return apiRenderPasses.GetLength(); }
	[[nodiscard]] uint8 GetSubPassCount(const uint8 renderPass) const { return subPasses[renderPass].GetLength(); }

	Id GetSubPassName(const uint8 rp, const uint8 sp) { return subPasses[rp][sp].Name; }

	//Enables or disables render passes
	//Right now we have to guarantee enabledRenderPasses has the correct order
	//TODO: maybe everytime we have to execute a render pass check if is enabled so we don't have to keep order between the two collections
	void ToggleRenderPass(Id renderPassName, bool enable)
	{
		if(enable)
		{
			enabledRenderPasses.EmplaceBack(renderPassName);
		}
		else
		{
			for(uint8 i = 0; i < enabledRenderPasses.GetLength(); ++i)
			{
				if (enabledRenderPasses[i] == renderPassName) { enabledRenderPasses.Pop(i); break; }
			}
		}
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

	GTSL::Array<Id, 16> enabledRenderPasses;

	struct RenderPassData
	{
		GTSL::Array<Id, 8> RenderGroups;
		PassType PassType;
		uint8 APIRenderPass = 0, APISubPass = 0;
	};
	GTSL::FlatHashMap<RenderPassData, BE::PAR> renderPassesMap;
	GTSL::Array<Id, 8> renderPassesNames;

	using RenderPassFunctionType = GTSL::FunctionPointer<void(GameInstance*, RenderSystem*, MaterialSystem*, CommandBuffer, Id)>;
	
	GTSL::StaticMap<RenderPassFunctionType, 8> renderPassesFunctions;

	void renderScene(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);
	void renderUI(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);
	void renderRays(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp);

	struct RenderPassAttachment
	{
		TextureLayout Layout;
		uint8 Index;
	};

	struct APIRenderPassData
	{
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
	Texture GetAttachmentTexture(const Id attachment) const { return attachments.At(attachment()).Texture; }
	TextureView GetAttachmentTextureView(const Id attachment) const { return attachments.At(attachment()).TextureView; }
	TextureSampler GetAttachmentTextureSampler(const Id attachment) const { return attachments.At(attachment()).TextureSampler; }
};
