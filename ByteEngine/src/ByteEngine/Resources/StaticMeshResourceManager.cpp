#include "StaticMeshResourceManager.h"

#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>
#include <assimp/scene.h>
#include <GTSL/System.h>

StaticMeshResourceData* StaticMeshResourceManager::TryGetResource(const GTSL::String& name)
{
	GTSL::Id64 hashed_name(name);
	
	{
		resourceMapMutex.ReadLock();
		if (resources.contains(hashed_name))
		{
			resourceMapMutex.ReadUnlock();
			resourceMapMutex.WriteLock();
			auto& res = resources.at(hashed_name);
			res.IncrementReferences();
			resourceMapMutex.WriteUnlock();
			return &res;
		}
		resourceMapMutex.ReadUnlock();
	}

	StaticMeshResourceData data;

	Assimp::Importer importer;

	GTSL::String path(512, &transientAllocator);
	GTSL::System::GetRunningPath(path);
	path += "resources/";
	path += name;
	path += '.';
	path += "obj";

	const auto ai_scene = importer.ReadFile(path.c_str(), aiProcess_Triangulate | aiProcess_FlipUVs | aiProcess_JoinIdenticalVertices | aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_ImproveCacheLocality);

	if (!ai_scene || ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE || !ai_scene->mRootNode)
	{
		auto res = importer.GetErrorString();
		return nullptr;
	}

	auto InMesh = ai_scene->mMeshes[0];

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

		data.VertexArray[i].Tangent.X = InMesh->mTangents[i].x;
		data.VertexArray[i].Tangent.Y = InMesh->mTangents[i].y;
		data.VertexArray[i].Tangent.Z = InMesh->mTangents[i].z;

		data.VertexArray[i].BiTangent.X = InMesh->mBitangents[i].x;
		data.VertexArray[i].BiTangent.Y = InMesh->mBitangents[i].y;
		data.VertexArray[i].BiTangent.Z = InMesh->mBitangents[i].z;
	}

	//We allocate a new array of unsigned ints big enough to hold the number of indices in this mesh and assign it to the
	//pointer found inside the mesh.
	data.IndexArray = new uint16[InMesh->mNumFaces * 3];

	//Wow loop through each of the mesh's faces and retrieve the corresponding vertex indices.
	for (uint32 f = 0; f < InMesh->mNumFaces; ++f)
	{
		const auto Face = InMesh->mFaces[f];

		// Retrieve all indices of the face and store them in the indices array.
		for (uint32 i = 0; i < Face.mNumIndices; i++)
		{
			data.IndexArray[data.IndexCount + i] = Face.mIndices[i];
		}

		//Update the vertex count by summing the number of indices that each face we loop through has.
		data.IndexCount += Face.mNumIndices;
	}

	resourceMapMutex.WriteLock();
	resources.emplace(hashed_name, GTSL::MakeTransferReference(data)).first->second.IncrementReferences();
	resourceMapMutex.WriteUnlock();
	return nullptr;

}
