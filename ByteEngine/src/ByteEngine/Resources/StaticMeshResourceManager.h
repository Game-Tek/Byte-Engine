#pragma once

#include "SubResourceManager.h"
#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>

#include <GTSL/DynamicType.h>
#include "ResourceData.h"

class StaticMeshResourceManager final : public SubResourceManager
{
public:
	StaticMeshResourceManager() : SubResourceManager("Static Mesh")
	{
	}

	struct OnStaticMeshLoad
	{
		SAFE_POINTER UserData;
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
		SAFE_POINTER UserData;
		GTSL::Ranger<byte> MeshDataBuffer;
		GTSL::Delegate<void(OnStaticMeshLoad)> OnStaticMeshLoad;
		uint32 IndicesAlignment = 0;
	};
	void LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo);

	void GetMeshSize(const GTSL::StaticString<256>& name, uint32& meshSize);
	
private:
	GTSL::FlatHashMap<OnStaticMeshLoad> resources;
};
