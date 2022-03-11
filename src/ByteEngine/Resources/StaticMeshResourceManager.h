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

		Array<Array<GAL::ShaderDataType, 8>, 8>& GetVertexDescriptor() { return VertexDescriptor; }
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
	void LoadStaticMeshInfo(ApplicationManager* gameInstance, Id meshName, TaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->EnqueueTask(gameInstance->RegisterTask(this, u8"StaticMeshResourceManager::loadStaticMeshInfo", {}, &StaticMeshResourceManager::loadStaticMeshInfo<ARGS...>, {}, {}), GTSL::MoveRef(meshName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadStaticMesh(ApplicationManager* gameInstance, StaticMeshInfo staticMeshInfo, uint32 verticesOffset, GTSL::Range<byte*> vertexBuffer, uint32 indicesOffset, GTSL::Range<byte*> indexBuffer, TaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->EnqueueTask(gameInstance->RegisterTask(this, u8"StaticMeshResourceManager::loadStaticMesh", {}, &StaticMeshResourceManager::loadMesh<ARGS...>, {}, {}), GTSL::MoveRef(staticMeshInfo), GTSL::MoveRef(verticesOffset), GTSL::MoveRef(vertexBuffer), GTSL::MoveRef(indicesOffset), GTSL::MoveRef(indexBuffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
private:
	ResourceFiles resource_files_;

	template<typename... ARGS>
	void loadStaticMeshInfo(TaskInfo taskInfo, Id meshName, TaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS... args)
	{
		StaticMeshInfo static_mesh_info;
		resource_files_.LoadEntry(meshName, static_mesh_info);

		taskInfo.ApplicationManager->EnqueueTask(dynamicTaskHandle, GTSL::MoveRef(static_mesh_info), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadMesh(TaskInfo taskInfo, StaticMeshInfo staticMeshInfo, uint32 verticesOffset, GTSL::Range<byte*> vertexBuffer, uint32 indicesOffset, GTSL::Range<byte*> indexBuffer, TaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS... args)
	{
		auto verticesSize = staticMeshInfo.GetVertexSize() * staticMeshInfo.GetVertexCount(); auto indicesSize = staticMeshInfo.GetIndexSize() * staticMeshInfo.GetIndexCount();

		BE_ASSERT(vertexBuffer.Bytes() >= verticesSize, u8"")
		BE_ASSERT(indexBuffer.Bytes() >= indicesSize, u8"")

		byte* vertices = vertexBuffer.begin();
		byte* indices = indexBuffer.begin();

		const auto bufferSize = vertexBuffer.Bytes(), verticesThatFitInBuffer = (bufferSize / staticMeshInfo.GetVertexSize());

		for(auto i = 0u, offsetReadInFile = 0u, accumulatedVertexOffset = 0u; i < staticMeshInfo.GetVertexDescriptor().Length; ++i) {
			auto streamSize = GAL::GraphicsPipeline::GetVertexSize(staticMeshInfo.GetVertexDescriptor().array[i]);
			resource_files_.LoadData(staticMeshInfo, { staticMeshInfo.GetVertexCount() * streamSize, vertices + accumulatedVertexOffset * verticesThatFitInBuffer + verticesOffset * streamSize }, offsetReadInFile, streamSize * staticMeshInfo.GetVertexCount());
			offsetReadInFile += streamSize * staticMeshInfo.GetVertexCount();
			accumulatedVertexOffset += streamSize;
		}

		resource_files_.LoadData(staticMeshInfo, { staticMeshInfo.GetIndexCount() * 2u/*todo: use actual byte offset*/, indexBuffer.begin() + indicesOffset * 2u/*todo: use actual index byte offset*/}, verticesSize, indicesSize);

		taskInfo.ApplicationManager->EnqueueTask(dynamicTaskHandle, GTSL::MoveRef(staticMeshInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	bool loadMesh(const GTSL::Buffer<BE::TAR>& sourceBuffer, StaticMeshInfo& meshInfo, GTSL::Buffer<BE::TAR>& meshDataBuffer, const GTSL::StringView fileExtension);
};
