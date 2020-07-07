#pragma once

#include "SubResourceManager.h"
#include <GTSL/Delegate.hpp>

#include "ResourceData.h"
#include <GTSL/Id.h>

class StaticMeshResourceManager final : public SubResourceManager
{
public:
	StaticMeshResourceManager() : SubResourceManager("Static Mesh")
	{
	}

	struct OnStaticMeshLoad
	{
		/**
		 * \brief Buffer containing the loaded data. At the start all vertices are found and after VertexCount vertices the indeces are found.
		 */
		GTSL::Ranger<byte> MeshDataBuffer;
		/**
		 * \brief Number of vertices the loaded mesh contains.
		 */
		GTSL::uint32 VertexCount;
		/**
		 * \brief Number of indeces the loaded mesh contains. Every face can only have three indeces.
		 */
		GTSL::uint16 IndexCount;
		void* Vertex = nullptr;
		void* Indices = nullptr;
	};

	struct LoadStaticMeshInfo : ResourceLoadInfo
	{
		GTSL::Ranger<byte> MeshDataBuffer;
		GTSL::Delegate<void(OnStaticMeshLoad)> OnStaticMeshLoad;
	};
	void LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo);
	
private:
	//std::unordered_map<GTSL::Id64::HashType, StaticMeshResourceData> resources;
};
