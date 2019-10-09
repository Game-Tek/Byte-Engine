#pragma once

#include "Core.h"

#include "Resource.h"

#include "Vertex.h"

//Used to specify a single mesh. Contains a pointer to an array of vertices, and a pointer to an array of indices.
struct Model
{
	//Pointer to Vertex Array.
	Vertex * VertexArray = nullptr;
	//Pointer to index array.
	uint16 * IndexArray = nullptr;

	//Vertex Count.
	uint16 VertexCount = 0;
	//Index Count.
	uint16 IndexCount = 0;
};

class FString;

struct aiNode;
struct aiMesh;
struct aiScene;

class VertexDescriptor;

class GS_API StaticMeshResource final : public Resource
{
public:
	StaticMeshResource() = default;
	~StaticMeshResource();

	[[nodiscard]] size_t GetDataSize() const override { return SCAST(Model*, Data)->IndexCount * sizeof(uint16) + SCAST(Model*, Data)->VertexCount * sizeof(Vertex); }
	[[nodiscard]] Model* GetModel() const { return SCAST(Model*, Data); }

	static VertexDescriptor* GetVertexDescriptor();
	bool LoadResource(const FString& _Path) override;
private:
	void LoadFallbackResource(const FString& _Path) override;

	static Model * ProcessNode(aiNode * Node, const aiScene * Scene);
	static Model ProcessMesh(aiMesh * Mesh);

	static VertexDescriptor StaticMeshVertexTypeVertexDescriptor;
};