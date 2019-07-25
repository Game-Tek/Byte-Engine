#pragma once

#include "Core.h"

#include "Resource.h"

#include "Vertex.h"

//Used to specify a single mesh. Contains a pointer to an array of vertices, and a pointer to an array of indices.
struct Mesh
{
	//Pointer to Vertex Array.
	Vertex * VertexArray = nullptr;
	//Pointer to index array.
	uint32 * IndexArray = nullptr;

	//Vertex Count.
	uint32 VertexCount = 0;
	//Index Count.
	uint32 IndexCount = 0;
};

class FString;

struct aiNode;
struct aiMesh;
struct aiScene;

GS_CLASS StaticMeshResource : public Resource
{
public:
	explicit StaticMeshResource(const FString & Path);
	~StaticMeshResource();

	Vertex * GetVertexArray() const { return Data->VertexArray; }
	uint32 * GetIndexArray() const { return Data->IndexArray; }

	size_t GetVertexArraySize() const { return Data->VertexCount * sizeof(Vertex); }
	size_t GetIndexArraySize() const { return Data->IndexCount * sizeof(uint32); }

	size_t GetDataSize() const override { return sizeof(*Data); }

	uint32 GetMeshIndexCount(uint8 MeshIndex) const { return Data->IndexCount; };
	uint32 GetMeshVertexCount(uint8 MeshIndex) const { return Data->VertexCount; }

private:
	Mesh * Data;

	Mesh * Load(const FString & Path);
	Mesh * LoadFallbackResource() const;
	Mesh * ProcessNode(aiNode * Node, const aiScene * Scene);
	Mesh ProcessMesh(aiMesh * Mesh);
};