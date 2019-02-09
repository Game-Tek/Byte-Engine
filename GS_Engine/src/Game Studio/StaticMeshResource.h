#pragma once

#include "Core.h"

#include "Resource.h"

#include "Vertex.h"

//Used to specify a single mesh. Contains a pointer to an array of vertices, and a pointer to an array of indices.
struct Mesh
{
	Vertex * VertexArray = nullptr;
	uint32 * IndexArray = nullptr;

	uint32 VertexCount = 0;
	uint32 IndexCount = 0;
};

#include "assimp/scene.hpp"

GS_CLASS StaticMeshResource : public Resource
{
public:
	StaticMeshResource(const std::string & Path);
	~StaticMeshResource();

	Mesh * GetMeshData() const { return static_cast<Mesh *>(Data); }
	size_t GetDataSize() const override { return sizeof(*static_cast<Mesh*>(Data)); }

	uint32 GetMeshIndexCount(uint8 MeshIndex) const { return static_cast<Mesh *>(Data)[MeshIndex].IndexCount; };
	uint32 GetMeshVertexCount(uint8 MeshIndex) const { return static_cast<Mesh *>(Data)[MeshIndex].VertexCount; }

private:
	Mesh * Load(const char * FilePath);
	Mesh * LoadFallbackResource();
	Mesh ** ProcessNode(aiNode * Node, const aiScene * Scene);
	Mesh * ProcessMesh(aiMesh * Mesh);
};