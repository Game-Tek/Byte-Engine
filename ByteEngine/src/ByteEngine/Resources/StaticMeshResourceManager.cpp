#include "StaticMeshResourceManager.h"

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>
#include <assimp/scene.h>

#include <GTSL/Buffer.h>
#include <GAL/RenderCore.h>
#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.h>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Game/GameInstance.h"

StaticMeshResourceManager::StaticMeshResourceManager() : ResourceManager("StaticMeshResourceManager"), meshInfos(4, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, package_path, resources_path, index_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	package_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	index_path += BE::Application::Get()->GetPathToApplication();
	query_path += "/resources/*.obj";
	package_path += "/resources/StaticMeshes.bepkg";
	index_path += "/resources/StaticMeshes.beidx";
	resources_path += "/resources/";

	indexFile.OpenFile(index_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	staticMeshPackage.OpenFile(package_path, (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::LEAVE_CONTENTS);
	
	GTSL::Buffer file_buffer; file_buffer.Allocate(2048 * 2048, 32, GetTransientAllocator());

	if (indexFile.ReadFile(file_buffer))
	{
		GTSL::Extract(meshInfos, file_buffer);
	}
	
	auto load = [&](const GTSL::FileQuery::QueryResult& queryResult)
	{
		auto file_path = resources_path;
		file_path += queryResult.FileNameWithExtension;
		auto name = queryResult.FileNameWithExtension; name.Drop(name.FindLast('.'));
		const auto hashed_name = GTSL::Id64(name);

		if (!meshInfos.Find(hashed_name))
		{
			GTSL::File query_file;
			query_file.OpenFile(file_path, static_cast<uint8>(GTSL::File::AccessMode::READ), GTSL::File::OpenMode::LEAVE_CONTENTS);

			query_file.ReadFile(file_buffer);

			MeshInfo mesh_info; Mesh mesh(1024, GetTransientAllocator());

			loadMesh(file_buffer, mesh_info, mesh);

			mesh_info.ByteOffset = static_cast<uint32>(staticMeshPackage.GetFileSize());

			Insert(mesh, file_buffer);
			staticMeshPackage.WriteToFile(file_buffer);

			meshInfos.Emplace(hashed_name, mesh_info);

			query_file.CloseFile();
		}
	};
	
	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, load);

	indexFile.CloseFile();
	indexFile.OpenFile(index_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::CLEAR);

	file_buffer.Resize(0);
	Insert(meshInfos, file_buffer);
	indexFile.WriteToFile(file_buffer);
	
	file_buffer.Free(32, GetTransientAllocator());
}

StaticMeshResourceManager::~StaticMeshResourceManager()
{
	staticMeshPackage.CloseFile(); indexFile.CloseFile();
}

void StaticMeshResourceManager::LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo)
{
	const auto meshInfo = meshInfos.At(loadStaticMeshInfo.Name);

	staticMeshPackage.SetPointer(meshInfo.ByteOffset, GTSL::File::MoveFrom::BEGIN);

	byte* vertices = loadStaticMeshInfo.DataBuffer, *indices = static_cast<byte*>(GTSL::AlignPointer(loadStaticMeshInfo.IndicesAlignment, vertices + meshInfo.VerticesSize));
	
	staticMeshPackage.ReadFromFile(loadStaticMeshInfo.DataBuffer); 

	GTSL::MemCopy(meshInfo.IndecesSize, vertices + meshInfo.VerticesSize, indices);

	const auto mesh_size = (indices + meshInfo.IndecesSize) - vertices;
	
	OnStaticMeshLoad on_static_mesh_load;
	on_static_mesh_load.IndexCount = meshInfo.IndecesSize;
	on_static_mesh_load.VertexCount = meshInfo.VerticesSize;
	on_static_mesh_load.DataBuffer = GTSL::Ranger<byte>(mesh_size, loadStaticMeshInfo.DataBuffer.begin());
	loadStaticMeshInfo.GameInstance->AddDynamicTask("OnStaticMeshLoad", loadStaticMeshInfo.OnStaticMeshLoad, loadStaticMeshInfo.ActsOn,
		loadStaticMeshInfo.StartOn, loadStaticMeshInfo.DoneFor, GTSL::MoveRef(on_static_mesh_load));
}

void StaticMeshResourceManager::GetMeshSize(const GTSL::Id64 name, const uint32 alignment, uint32& meshSize)
{
	auto& mesh = meshInfos.At(name);
	meshSize = GTSL::Math::PowerOf2RoundUp(mesh.VerticesSize, alignment) + mesh.IndecesSize;
}

void StaticMeshResourceManager::loadMesh(const GTSL::Buffer& sourceBuffer, MeshInfo& meshInfo, Mesh& mesh)
{
	Assimp::Importer importer;
	const auto* const ai_scene = importer.ReadFileFromMemory(sourceBuffer.GetData(), sourceBuffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
		aiProcess_JoinIdenticalVertices | aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_ImproveCacheLocality);

	BE_ASSERT(ai_scene != nullptr && !(ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE), "Error interpreting file!");

	aiMesh* in_mesh = ai_scene->mMeshes[0];

	//MESH ALWAYS HAS POSITIONS
	meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT3));
	for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
	{
		mesh.VertexElements.EmplaceBack(in_mesh->mVertices[vertex].x);
		mesh.VertexElements.EmplaceBack(in_mesh->mVertices[vertex].y);
		mesh.VertexElements.EmplaceBack(in_mesh->mVertices[vertex].z);
	}
	meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;

	if (in_mesh->HasNormals())
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT3));

		for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
		{
			mesh.VertexElements.EmplaceBack(in_mesh->mNormals[vertex].x);
			mesh.VertexElements.EmplaceBack(in_mesh->mNormals[vertex].y);
			mesh.VertexElements.EmplaceBack(in_mesh->mNormals[vertex].z);

		}

		meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;
	}

	if (in_mesh->HasTangentsAndBitangents())
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT3));
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT3));

		for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
		{
			mesh.VertexElements.EmplaceBack(in_mesh->mTangents[vertex].x);
			mesh.VertexElements.EmplaceBack(in_mesh->mTangents[vertex].y);
			mesh.VertexElements.EmplaceBack(in_mesh->mTangents[vertex].z);
			mesh.VertexElements.EmplaceBack(in_mesh->mBitangents[vertex].x);
			mesh.VertexElements.EmplaceBack(in_mesh->mBitangents[vertex].y);
			mesh.VertexElements.EmplaceBack(in_mesh->mBitangents[vertex].z);
		}
		
		meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices * 2;
	}

	for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(in_mesh->GetNumUVChannels()); ++tex_coords)
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT2));

		for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
		{
			mesh.VertexElements.EmplaceBack(in_mesh->mTextureCoords[tex_coords][vertex].x);
			mesh.VertexElements.EmplaceBack(in_mesh->mTextureCoords[tex_coords][vertex].y);
		}

		meshInfo.VerticesSize += sizeof(GTSL::Vector2) * in_mesh->mNumVertices;
	}

	for (uint8 colors = 0; colors < static_cast<uint8>(in_mesh->GetNumColorChannels()); ++colors)
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT4));
		
		for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
		{
			mesh.VertexElements.EmplaceBack(in_mesh->mColors[colors][vertex].r);
			mesh.VertexElements.EmplaceBack(in_mesh->mColors[colors][vertex].g);
			mesh.VertexElements.EmplaceBack(in_mesh->mColors[colors][vertex].b);
			mesh.VertexElements.EmplaceBack(in_mesh->mColors[colors][vertex].a);
		}
		
		meshInfo.VerticesSize += sizeof(GTSL::Vector4) * in_mesh->mNumVertices;
	}

	for (uint32 face = 0; face < in_mesh->mNumFaces; ++face)
	{
		for (uint32 index = 0; index < in_mesh->mFaces[face].mNumIndices; ++index)
		{
			mesh.Indeces.EmplaceBack(in_mesh->mFaces[face].mIndices[index]);
		}
	}

	meshInfo.IndecesSize = in_mesh->mNumFaces * 3 * sizeof(uint32);
}

void Insert(const StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer)
{
	GTSL::Insert(meshInfo.VerticesSize, buffer);
	GTSL::Insert(meshInfo.IndecesSize, buffer);
	GTSL::Insert(meshInfo.ByteOffset, buffer);
}

void Extract(StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer)
{
	GTSL::Extract(meshInfo.VerticesSize, buffer);
	GTSL::Extract(meshInfo.IndecesSize, buffer);
	GTSL::Extract(meshInfo.ByteOffset, buffer);
}

void Insert(const StaticMeshResourceManager::Mesh& mesh, GTSL::Buffer& buffer)
{
	buffer.WriteBytes(mesh.VertexElements.GetLengthSize(), (byte*)mesh.VertexElements.begin());
	buffer.WriteBytes(mesh.Indeces.GetLengthSize(), (byte*)mesh.Indeces.begin());
}