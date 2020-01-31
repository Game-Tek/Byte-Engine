#pragma once

#include "Core.h"

#include "Resource.h"

#include "Vertex.h"

class FString;

struct aiNode;
struct aiMesh;
struct aiScene;

class VertexDescriptor;

struct Model
{
	//Pointer to Vertex Array.
	Vertex* VertexArray = nullptr;
	//Pointer to index array.
	uint16* IndexArray = nullptr;

	//Vertex Count.
	uint16 VertexCount = 0;
	//Index Count.
	uint16 IndexCount = 0;
};

class StaticMeshResource final : public Resource
{
public:
	//Used to specify a single mesh. Contains a pointer to an array of vertices, and a pointer to an array of indices.
	class StaticMeshResourceData final : public ResourceData
	{
	public:
		//Pointer to Vertex Array.
		Vertex* VertexArray = nullptr;
		//Pointer to index array.
		uint16* IndexArray = nullptr;

		//Vertex Count.
		uint16 VertexCount = 0;
		//Index Count.
		uint16 IndexCount = 0;

		~StaticMeshResourceData()
		{
			delete[] VertexArray;
			delete[] IndexArray;
		}
	};

private:
	StaticMeshResourceData data;

	bool loadResource(const LoadResourceData& LRD_) override;
	void loadFallbackResource(const FString& _Path) override;
	[[nodiscard]] const char* getResourceTypeExtension() const override { return "obj"; }

	static StaticMeshResourceData* ProcessNode(aiNode* Node, const aiScene* Scene);
	static StaticMeshResourceData ProcessMesh(aiMesh* Mesh);

	static RAPI::VertexDescriptor StaticMeshVertexTypeVertexDescriptor;

public:
	StaticMeshResource() = default;
	~StaticMeshResource() = default;

	[[nodiscard]] const char* GetName() const override { return "Static Mesh Resource"; }

	[[nodiscard]] const StaticMeshResourceData& GetStaticMeshData() const { return data; }

	static RAPI::VertexDescriptor* GetVertexDescriptor();
};
