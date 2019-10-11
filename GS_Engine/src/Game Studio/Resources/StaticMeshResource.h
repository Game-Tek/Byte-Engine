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

class GS_API StaticMeshResource final : public Resource
{
public:
	//Used to specify a single mesh. Contains a pointer to an array of vertices, and a pointer to an array of indices.
	class StaticMeshResourceData : public ResourceData
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

		void** WriteTo(size_t _Index, size_t _Bytes) override;
	};

	StaticMeshResource() = default;
	~StaticMeshResource();

	[[nodiscard]] const char* GetName() const override { return "Static Mesh Resource"; }
	[[nodiscard]] const char* GetResourceTypeExtension() const override { return ".gssm"; }

	[[nodiscard]] Model GetModel() const
	{
		return Model { SCAST(StaticMeshResourceData*, Data)->VertexArray, SCAST(StaticMeshResourceData*, Data)->IndexArray, SCAST(StaticMeshResourceData*, Data)->VertexCount, SCAST(StaticMeshResourceData*, Data)->IndexCount };
	}

	static VertexDescriptor* GetVertexDescriptor();
	bool LoadResource(const FString& _Path) override;
private:
	void LoadFallbackResource(const FString& _Path) override;

	static StaticMeshResourceData * ProcessNode(aiNode * Node, const aiScene * Scene);
	static StaticMeshResourceData ProcessMesh(aiMesh * Mesh);

	static VertexDescriptor StaticMeshVertexTypeVertexDescriptor;
};