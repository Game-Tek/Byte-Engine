#include "StaticMeshResourceManager.h"

#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>
#include <assimp/scene.h>

bool StaticMeshResourceManager::LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
	StaticMeshResourceData data;

	//Create Importer.
	Assimp::Importer Importer;

	//Create Scene and import file.
	const aiScene* Scene = Importer.ReadFile(loadResourceInfo.ResourcePath.c_str(),	aiProcess_Triangulate | aiProcess_FlipUVs | aiProcess_JoinIdenticalVertices	| aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals |	aiProcess_ImproveCacheLocality);

	if (!Scene || Scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE || !Scene->mRootNode)
	{
		auto Res = Importer.GetErrorString();
		return false;
	}

	aiMesh* InMesh = Scene->mMeshes[0];

	data.VertexArray = new Vertex[InMesh->mNumVertices];
	//Set this mesh's vertex count as the number of vertices found in this mesh.
	data.VertexCount = InMesh->mNumVertices;

	//------------MESH SETUP------------

	// Loop through each vertex.
	for (uint32 i = 0; i < InMesh->mNumVertices; i++)
	{
		// Positions
		data.VertexArray[i].Position.X = InMesh->mVertices[i].x;
		data.VertexArray[i].Position.Y = InMesh->mVertices[i].y;
		data.VertexArray[i].Position.Z = InMesh->mVertices[i].z;

		// Normals
		data.VertexArray[i].Normal.X = InMesh->mNormals[i].x;
		data.VertexArray[i].Normal.Y = InMesh->mNormals[i].y;
		data.VertexArray[i].Normal.Z = InMesh->mNormals[i].z;

		// Texture Coordinates
		if (InMesh->mTextureCoords[0]) //We check if the pointer to texture coords is valid. (Could be NULLPTR)
		{
			//A vertex can contain up to 8 different texture coordinates.
			//Here, we are making the assumption we won't be using a model with more than one texture coordinates.
			data.VertexArray[i].TextCoord.U = InMesh->mTextureCoords[0][i].x;
			data.VertexArray[i].TextCoord.V = InMesh->mTextureCoords[0][i].y;
		}

		// Tangent
		//Result.VertexArray[i].Tangent.X = InMesh->mTangents[i].x;
		//Result.VertexArray[i].Tangent.Y = InMesh->mTangents[i].y;
		//Result.VertexArray[i].Tangent.Z = InMesh->mTangents[i].z;

		/*
		// BiTangent
		Result.VertexArray[i].BiTangent.X = InMesh->mBitangents[i].x;
		Result.VertexArray[i].BiTangent.Y = InMesh->mBitangents[i].y;
		Result.VertexArray[i].BiTangent.Z = InMesh->mBitangents[i].z;
		*/
	}

	//We allocate a new array of unsigned ints big enough to hold the number of indices in this mesh and assign it to the
	//pointer found inside the mesh.
	data.IndexArray = new uint16[InMesh->mNumFaces * 3];

	//Wow loop through each of the mesh's faces and retrieve the corresponding vertex indices.
	for (uint32 f = 0; f < InMesh->mNumFaces; f++)
	{
		const aiFace Face = InMesh->mFaces[f];

		// Retrieve all indices of the face and store them in the indices array.
		for (uint32 i = 0; i < Face.mNumIndices; i++)
		{
			data.IndexArray[data.IndexCount + i] = Face.mIndices[i];
		}

		//Update the vertex count by summing the number of indices that each face we loop through has.
		data.IndexCount += Face.mNumIndices;
	}

	resources.insert({ loadResourceInfo.ResourceName, data });
	
	return true;
}

void StaticMeshResourceManager::LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
}

ResourceData* StaticMeshResourceManager::GetResource(const Id& name) { return &resources[name]; }

void StaticMeshResourceManager::ReleaseResource(const Id& resourceName) { if (resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName); } }