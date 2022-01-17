#pragma once

#include <GTSL/Buffer.hpp>

#include "ResourceManager.h"

#include <GTSL/MappedFile.hpp>
#include <GTSL/Math/Vectors.hpp>

#include "ByteEngine/Game/ApplicationManager.h"
#include "GAL/Pipelines.h"

namespace GAL {
	enum class ShaderDataType : unsigned char;
}

namespace GTSL {
	class Vector2;
}

class StaticMeshResourceManager final : public ResourceManager
{
public:
	StaticMeshResourceManager(const InitializeInfo&);
	~StaticMeshResourceManager();

	struct StaticMeshInfo : SData {
		struct SubMeshData : SubData {
			/**
			* \brief Number of vertices the loaded mesh contains.
			*/
			DEFINE_MEMBER(uint32, VertexCount)

			/**
			 * \brief Number of indeces the loaded mesh contains. Every face can only have three indeces.
			 */
			DEFINE_MEMBER(uint32, IndexCount)
			DEFINE_MEMBER(uint32, MaterialIndex)
			DEFINE_MEMBER(GTSL::Vector3, BoundingBox)
			DEFINE_MEMBER(float32, BoundingRadius)
			DEFINE_MEMBER(GTSL::ShortString<32>, ShaderGroupName);
		};

		DEFINE_MEMBER(uint32, VertexCount)
		DEFINE_MEMBER(uint32, IndexCount)

		/**
		 * \brief Size of a single index to determine whether to use uint16 or uint32.
		 */
		DEFINE_MEMBER(uint8, IndexSize)
		DEFINE_MEMBER(GTSL::Vector3, BoundingBox)
		DEFINE_MEMBER(float32, BoundingRadius)
		Array<Array<GAL::ShaderDataType, 8>, 8> VertexDescriptor;
		DEFINE_ARRAY_MEMBER(SubMeshData, SubMeshes, 16)
		DEFINE_MEMBER(bool, Interleaved)

		auto& GetVertexDescriptor() { return VertexDescriptor; }
		const auto& GetVertexDescriptor() const { return VertexDescriptor; }

		uint32 GetVertexSize() const {
			uint32 size = 0;

			for (auto& element : GetVertexDescriptor().array) {
				size += GAL::GraphicsPipeline::GetVertexSize(element);
			}

			return size;
		}
	};

	template<typename... ARGS>
	void LoadStaticMeshInfo(ApplicationManager* gameInstance, Id meshName, DynamicTaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"StaticMeshResourceManager::loadStaticMeshInfo", {}, &StaticMeshResourceManager::loadStaticMeshInfo<ARGS...>, {}, {}, GTSL::MoveRef(meshName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadStaticMesh(ApplicationManager* gameInstance, StaticMeshInfo staticMeshInfo, uint32 indicesAlignment, GTSL::Range<byte*> buffer, DynamicTaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"StaticMeshResourceManager::loadStaticMesh", {}, &StaticMeshResourceManager::loadMesh<ARGS...>, {}, {}, GTSL::MoveRef(staticMeshInfo), GTSL::MoveRef(indicesAlignment), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
private:
	ResourceFiles resource_files_;

	template<typename... ARGS>
	void loadStaticMeshInfo(TaskInfo taskInfo, Id meshName, DynamicTaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS... args)
	{
		StaticMeshInfo static_mesh_info;
		resource_files_.LoadEntry(meshName, static_mesh_info);

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(static_mesh_info), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadMesh(TaskInfo taskInfo, StaticMeshInfo staticMeshInfo, uint32 indicesAlignment, GTSL::Range<byte*> buffer, DynamicTaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS... args)
	{
		auto verticesSize = staticMeshInfo.GetVertexSize() * staticMeshInfo.GetVertexCount(); auto indicesSize = staticMeshInfo.GetIndexSize() * staticMeshInfo.GetIndexCount();

		BE_ASSERT(buffer.Bytes() >= GTSL::Math::RoundUpByPowerOf2(verticesSize, indicesAlignment) + indicesSize, u8"")

		byte* vertices = buffer.begin();
		byte* indices = GTSL::AlignPointer(indicesAlignment, vertices + verticesSize);

		resource_files_.LoadData(staticMeshInfo, buffer); //TODO: CUSTOM LOGIC

		//GTSL::MemCopy(verticesSize, mappedFile.GetData() + staticMeshInfo.ByteOffset, vertices);
		//GTSL::MemCopy(indicesSize, mappedFile.GetData() + staticMeshInfo.ByteOffset + verticesSize, indices);

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(staticMeshInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	bool loadMesh(const GTSL::Buffer<BE::TAR>& sourceBuffer, StaticMeshInfo& meshInfo, GTSL::Buffer<BE::TAR>& meshDataBuffer, const GTSL::StringView fileExtension);
};
