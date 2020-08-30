#include "StaticMeshResourceManager.h"

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>
#include <assimp/scene.h>
#include <GAL/Pipelines.h>

#include <GTSL/Buffer.h>
#include <GAL/RenderCore.h>
#include <GTSL/Filesystem.h>
#include <GTSL/Pair.h>
#include <GTSL/Serialize.h>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Game/GameInstance.h"

using ShaderDataTypeType = GTSL::UnderlyingType<GAL::ShaderDataType>;

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
	staticMeshPackage.OpenFile(package_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	
	GTSL::Buffer file_buffer; file_buffer.Allocate(2048 * 2048, 32, GetTransientAllocator());
	GTSL::Buffer mesh_buffer; mesh_buffer.Allocate(2048 * 2048, 32, GetTransientAllocator());

	if (indexFile.ReadFile(file_buffer))
	{
		GTSL::Extract(meshInfos, file_buffer);
		file_buffer.Free(32, GetTransientAllocator());
		mesh_buffer.Free(32, GetTransientAllocator());
		return;
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

			MeshInfo mesh_info;

			loadMesh(file_buffer, mesh_info, mesh_buffer); //writes into file buffer after reading, SAFE
			file_buffer.Resize(0);
			
			mesh_info.ByteOffset = static_cast<uint32>(staticMeshPackage.GetFileSize());

			staticMeshPackage.WriteToFile(mesh_buffer);

			meshInfos.Emplace(hashed_name, mesh_info);

			query_file.CloseFile();
		}
	};

	
	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, load);

	file_buffer.Resize(0);
	Insert(meshInfos, file_buffer);
	
	indexFile.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
	indexFile.WriteToFile(file_buffer);
	
	file_buffer.Free(32, GetTransientAllocator());
	mesh_buffer.Free(32, GetTransientAllocator());
}

StaticMeshResourceManager::~StaticMeshResourceManager()
{
	staticMeshPackage.CloseFile(); indexFile.CloseFile();
}

void StaticMeshResourceManager::LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo)
{
	const auto meshInfo = meshInfos.At(loadStaticMeshInfo.Name);

	staticMeshPackage.SetPointer(meshInfo.ByteOffset, GTSL::File::MoveFrom::BEGIN);

	byte* vertices = loadStaticMeshInfo.DataBuffer;
	byte* indices = GTSL::AlignPointer(loadStaticMeshInfo.IndicesAlignment, vertices + meshInfo.VerticesSize);
	
	[[maybe_unused]] auto bytes_read = staticMeshPackage.ReadFromFile(GTSL::Ranger<byte>(meshInfo.VerticesSize, vertices));
	BE_ASSERT(bytes_read != 0, "Read 0 bytes!");
	bytes_read = staticMeshPackage.ReadFromFile(GTSL::Ranger<byte>(meshInfo.IndicesSize, indices));
	BE_ASSERT(bytes_read != 0, "Read 0 bytes!");

	const auto mesh_size = (indices + meshInfo.IndicesSize) - vertices;
		
	OnStaticMeshLoad on_static_mesh_load;
	on_static_mesh_load.VertexSize = GAL::GraphicsPipeline::GetVertexSize(GTSL::Ranger<const GAL::ShaderDataType>(meshInfo.VertexDescriptor.GetLength(), reinterpret_cast<const GAL::ShaderDataType*>(meshInfo.VertexDescriptor.begin())));
	on_static_mesh_load.IndexCount = meshInfo.IndicesSize / meshInfo.IndexSize;
	on_static_mesh_load.VertexCount = meshInfo.VerticesSize / on_static_mesh_load.VertexSize;
	on_static_mesh_load.IndicesOffset = indices - vertices;
	on_static_mesh_load.VertexDescriptor = GTSL::Ranger<const GAL::ShaderDataType>(meshInfo.VertexDescriptor.GetLength(), reinterpret_cast<const GAL::ShaderDataType*>(meshInfo.VertexDescriptor.begin()));
	on_static_mesh_load.IndexSize = meshInfo.IndexSize;
	on_static_mesh_load.UserData = loadStaticMeshInfo.UserData;
	on_static_mesh_load.DataBuffer = GTSL::Ranger<byte>(mesh_size, loadStaticMeshInfo.DataBuffer.begin());
	loadStaticMeshInfo.GameInstance->AddDynamicTask("OnStaticMeshLoad", loadStaticMeshInfo.OnStaticMeshLoad, loadStaticMeshInfo.ActsOn, GTSL::MoveRef(on_static_mesh_load));
}

void StaticMeshResourceManager::GetMeshSize(const GTSL::Id64 name, uint16* indexSize, const uint16* indicesAlignment, uint32* meshSize, uint32* indicesOffset)
{
	auto& mesh = meshInfos.At(name);
	*indexSize = mesh.IndexSize;
	*indicesOffset = GTSL::Math::PowerOf2RoundUp(mesh.VerticesSize, static_cast<uint32>(*indicesAlignment));
	*meshSize = *indicesOffset + mesh.IndicesSize;
}

void StaticMeshResourceManager::loadMesh(const GTSL::Buffer& sourceBuffer, MeshInfo& meshInfo, GTSL::Buffer& mesh)
{
	Assimp::Importer importer;
	const auto* const ai_scene = importer.ReadFileFromMemory(sourceBuffer.GetData(), sourceBuffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
		aiProcess_JoinIdenticalVertices | aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_ImproveCacheLocality);

	BE_ASSERT(ai_scene != nullptr && !(ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE), "Error interpreting file!");

	aiMesh* in_mesh = ai_scene->mMeshes[0];

	GTSL::Array<GTSL::Pair<void*, uint8>, 20> vertex_elements;
	
	//MESH ALWAYS HAS POSITIONS
	meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT3));
	vertex_elements.EmplaceBack(in_mesh->mVertices, sizeof(GTSL::Vector3));
	meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;

	if(in_mesh->HasNormals())
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT3));
		vertex_elements.EmplaceBack(in_mesh->mNormals, sizeof(GTSL::Vector3));
		meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;
	}

	if(in_mesh->HasTangentsAndBitangents())
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT3));
		vertex_elements.EmplaceBack(in_mesh->mTangents, sizeof(GTSL::Vector3));
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT3));
		vertex_elements.EmplaceBack(in_mesh->mBitangents, sizeof(GTSL::Vector3));
		
		meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;
		meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;
	}

	for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(in_mesh->GetNumUVChannels()); ++tex_coords)
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT2));

		vertex_elements.EmplaceBack(in_mesh->mTextureCoords[tex_coords], sizeof(GTSL::Vector2));
		
		meshInfo.VerticesSize += sizeof(GTSL::Vector2) * in_mesh->mNumVertices;
	}

	for (uint8 colors = 0; colors < static_cast<uint8>(in_mesh->GetNumColorChannels()); ++colors)
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataType::FLOAT4));

		vertex_elements.EmplaceBack(in_mesh->mColors[colors], sizeof(GTSL::Vector4));

		meshInfo.VerticesSize += sizeof(GTSL::Vector4) * in_mesh->mNumVertices;
	}
	
	for(uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
	{
		for(auto& e : vertex_elements)
		{
			mesh.WriteBytes(e.Second, static_cast<byte*>(e.First) + vertex * e.Second);
		}
	}

	uint16 index_size = 0;
	
	if((in_mesh->mNumFaces * 3) < 65535)
	{
		index_size = 2;

		for (uint32 face = 0; face < in_mesh->mNumFaces; ++face)
		{
			for (uint32 index = 0; index < in_mesh->mFaces[face].mNumIndices; ++index)
			{
				uint16 idx = static_cast<uint16>(in_mesh->mFaces[face].mIndices[index]);
				mesh.WriteBytes(index_size, reinterpret_cast<byte*>(&idx));
			}
		}
	}
	else
	{
		index_size = 4;

		for (uint32 face = 0; face < in_mesh->mNumFaces; ++face)
		{
			for (uint32 index = 0; index < in_mesh->mFaces[face].mNumIndices; ++index)
			{
				mesh.WriteBytes(index_size, reinterpret_cast<byte*>(in_mesh->mFaces[face].mIndices + index));
			}
		}
	}

	meshInfo.IndicesSize = in_mesh->mNumFaces * 3 * index_size;
	meshInfo.IndexSize = index_size;
}

void Insert(const StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer)
{
	GTSL::Insert(meshInfo.VertexDescriptor, buffer);
	GTSL::Insert(meshInfo.VerticesSize, buffer);
	GTSL::Insert(meshInfo.IndicesSize, buffer);
	GTSL::Insert(meshInfo.ByteOffset, buffer);
	GTSL::Insert(meshInfo.IndexSize, buffer);
}

void Extract(StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer)
{
	GTSL::Extract(meshInfo.VertexDescriptor, buffer);
	GTSL::Extract(meshInfo.VerticesSize, buffer);
	GTSL::Extract(meshInfo.IndicesSize, buffer);
	GTSL::Extract(meshInfo.ByteOffset, buffer);
	GTSL::Extract(meshInfo.IndexSize, buffer);
}