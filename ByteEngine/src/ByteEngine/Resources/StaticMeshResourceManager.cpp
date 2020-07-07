#include "StaticMeshResourceManager.h"

#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>
#include <assimp/scene.h>

#include "ByteEngine/Application/Application.h"
#include <new>

#include "ByteEngine/Vertex.h"

void StaticMeshResourceManager::LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo)
{
	GTSL::StaticString<1024> path;
	path += BE::Application::Get()->GetResourceManager()->GetResourcePath();
	path += loadStaticMeshInfo.Name;
	path += ".obj";

	GTSL::File file;
	file.OpenFile(path, GTSL::File::OpenFileMode::READ);
	auto file_size = file.GetFileSize();
	auto range = GTSL::Ranger<byte>(file_size, loadStaticMeshInfo.MeshDataBuffer.begin());
	file.ReadFromFile(range);

	Assimp::Importer importer;

	const auto* const ai_scene = importer.ReadFileFromMemory(range.begin(), range.Bytes(), aiProcess_Triangulate | aiProcess_FlipUVs | aiProcess_JoinIdenticalVertices | aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_ImproveCacheLocality);

	file.CloseFile();
	
	if (!ai_scene || ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE || !ai_scene->mRootNode)
	{
		auto res = importer.GetErrorString();
	}

	aiMesh* InMesh = ai_scene->mMeshes[0];

	auto vertices = ::new(range.begin()) Vertex[InMesh->mNumVertices];

	//------------MESH SETUP------------

	// Loop through each vertex.
	for (uint32 i = 0; i < InMesh->mNumVertices; i++)
	{
		// Positions
		vertices[i].Position.X = InMesh->mVertices[i].x;
		vertices[i].Position.Y = InMesh->mVertices[i].y;
		vertices[i].Position.Z = InMesh->mVertices[i].z;

		// Normals
		vertices[i].Normal.X = InMesh->mNormals[i].x;
		vertices[i].Normal.Y = InMesh->mNormals[i].y;
		vertices[i].Normal.Z = InMesh->mNormals[i].z;

		// Texture Coordinates
		if (InMesh->mTextureCoords[0]) //We check if the pointer to texture coords is valid. (Could be NULLPTR)
		{
			//A vertex can contain up to 8 different texture coordinates.
			//Here, we are making the assumption we won't be using a model with more than one texture coordinates.
			vertices[i].TextCoord.U() = InMesh->mTextureCoords[0][i].x;
			vertices[i].TextCoord.V() = InMesh->mTextureCoords[0][i].y;
		}

		//vertices[i].Tangent.X = InMesh->mTangents[i].x;
		//vertices[i].Tangent.Y = InMesh->mTangents[i].y;
		//vertices[i].Tangent.Z = InMesh->mTangents[i].z;
		//
		//vertices[i].BiTangent.X = InMesh->mBitangents[i].x;
		//vertices[i].BiTangent.Y = InMesh->mBitangents[i].y;
		//vertices[i].BiTangent.Z = InMesh->mBitangents[i].z;
	}

	auto normal = vertices + ai_scene->mMeshes[0]->mNumVertices;
	auto aligned = GTSL::AlignPointer(256, vertices + ai_scene->mMeshes[0]->mNumVertices);
	
	auto indeces = ::new(aligned) uint16[InMesh->mNumFaces * 3];
	uint32 index_count{ 0 };
	
	for (uint32 f = 0; f < InMesh->mNumFaces; ++f)
	{
		const auto Face = InMesh->mFaces[f];

		for (uint32 i = 0; i < Face.mNumIndices; i++) { indeces[f + i] = Face.mIndices[i]; }

		index_count += Face.mNumIndices;
	}

	OnStaticMeshLoad on_static_mesh_load;
	on_static_mesh_load.Vertex = vertices;
	on_static_mesh_load.Indices = indeces;
	on_static_mesh_load.IndexCount = index_count;
	on_static_mesh_load.VertexCount = InMesh->mNumVertices;
	on_static_mesh_load.MeshDataBuffer = GTSL::Ranger<byte>(range.begin(), reinterpret_cast<byte*>(indeces + index_count));
	loadStaticMeshInfo.OnStaticMeshLoad(on_static_mesh_load);
}
