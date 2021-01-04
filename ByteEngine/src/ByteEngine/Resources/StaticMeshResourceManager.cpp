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

	auto vertexSize = GAL::GraphicsPipeline::GetVertexSize(meshInfo.VertexDescriptor);
	auto verticesSize = meshInfo.VertexCount * vertexSize; auto indicesSize = meshInfo.IndexCount * meshInfo.IndexSize;
	
	byte* vertices = loadStaticMeshInfo.DataBuffer.begin();
	byte* indices = GTSL::AlignPointer(loadStaticMeshInfo.IndicesAlignment, vertices + verticesSize);
	
	[[maybe_unused]] auto bytes_read = staticMeshPackage.ReadFromFile(GTSL::Range<byte*>(verticesSize, vertices));
	BE_ASSERT(bytes_read != 0, "Read 0 bytes!");
	bytes_read = staticMeshPackage.ReadFromFile(GTSL::Range<byte*>(indicesSize, indices));
	BE_ASSERT(bytes_read != 0, "Read 0 bytes!");

	const auto mesh_size = verticesSize + indicesSize;
		
	OnStaticMeshLoad on_static_mesh_load;
	on_static_mesh_load.VertexCount = meshInfo.VertexCount;
	on_static_mesh_load.VertexSize = vertexSize;
	on_static_mesh_load.IndexCount = meshInfo.IndexCount;
	on_static_mesh_load.IndexSize = meshInfo.IndexSize;
	on_static_mesh_load.VertexDescriptor = meshInfo.VertexDescriptor;
	on_static_mesh_load.UserData = loadStaticMeshInfo.UserData;
	on_static_mesh_load.DataBuffer = GTSL::Range<byte*>(mesh_size, loadStaticMeshInfo.DataBuffer.begin());
	loadStaticMeshInfo.GameInstance->AddDynamicTask("onSMLoad", loadStaticMeshInfo.OnStaticMeshLoad, loadStaticMeshInfo.ActsOn, GTSL::MoveRef(on_static_mesh_load));
}

void StaticMeshResourceManager::GetMeshSize(const GTSL::Id64 name, uint32* vertexCount, uint32* vertexSize, uint32* indexCount, uint32* indexSize)
{
	auto& mesh = meshInfos.At(name);
	*vertexSize = 0;

	for(auto e : mesh.VertexDescriptor) { *vertexSize += GAL::ShaderDataTypesSize(e); }
	
	*vertexCount = mesh.VertexCount;
	*indexCount = mesh.IndexCount;
	*indexSize = mesh.IndexSize;
}

void StaticMeshResourceManager::loadMesh(const GTSL::Buffer& sourceBuffer, MeshInfo& meshInfo, GTSL::Buffer& mesh)
{
	Assimp::Importer importer;
	const auto* const ai_scene = importer.ReadFileFromMemory(sourceBuffer.GetData(), sourceBuffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
		aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_JoinIdenticalVertices);

	BE_ASSERT(ai_scene != nullptr && !(ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE), "Error interpreting file!");

	aiMesh* inMesh = ai_scene->mMeshes[0];

	//						ptr	  el.size jmp.size
	GTSL::Array<GTSL::Tuple<void*, uint8, uint8>, 20> vertexElements;
	
	meshInfo.VertexCount = inMesh->mNumVertices;
	
	//MESH ALWAYS HAS POSITIONS
	meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
	vertexElements.EmplaceBack(static_cast<void*>(inMesh->mVertices), sizeof(GTSL::Vector3), 12);

	if(inMesh->HasNormals())
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		vertexElements.EmplaceBack(static_cast<void*>(inMesh->mNormals), sizeof(GTSL::Vector3), 12);
	}

	if(inMesh->HasTangentsAndBitangents())
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		vertexElements.EmplaceBack(static_cast<void*>(inMesh->mTangents), sizeof(GTSL::Vector3), 12);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		vertexElements.EmplaceBack(static_cast<void*>(inMesh->mBitangents), sizeof(GTSL::Vector3), 12);
	}

	for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(inMesh->GetNumUVChannels()); ++tex_coords)
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT2);

		vertexElements.EmplaceBack(static_cast<void*>(inMesh->mTextureCoords[tex_coords]), sizeof(GTSL::Vector2), 12);
	}

	for (uint8 colors = 0; colors < static_cast<uint8>(inMesh->GetNumColorChannels()); ++colors)
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT4);

		vertexElements.EmplaceBack(static_cast<void*>(inMesh->mColors[colors]), sizeof(GTSL::Vector4), 16);
	}
	
	for(uint32 vertex = 0; vertex < inMesh->mNumVertices; ++vertex)
	{		
		for(auto& e : vertexElements)
		{
			mesh.WriteBytes(GTSL::Get<1>(e), static_cast<byte*>(GTSL::Get<0>(e)) + vertex * GTSL::Get<2>(e));
		}
	}

	uint16 indexSize = 0;
	
	if((inMesh->mNumFaces * 3) < 0xFFFF)
	{
		indexSize = 2;

		for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
			for (uint32 index = 0; index < 3; ++index) {
				uint16 idx = static_cast<uint16>(inMesh->mFaces[face].mIndices[index]);
				mesh.WriteBytes(indexSize, reinterpret_cast<byte*>(&idx));
			}
		}
	}
	else
	{
		indexSize = 4;

		for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
			for (uint32 index = 0; index < 3; ++index) {
				mesh.WriteBytes(indexSize, reinterpret_cast<byte*>(inMesh->mFaces[face].mIndices + index));
			}
		}
	}

	meshInfo.IndexCount = inMesh->mNumFaces * 3;
	meshInfo.IndexSize = indexSize;
}

void Insert(const StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer)
{
	GTSL::Insert(meshInfo.VertexDescriptor, buffer);
	GTSL::Insert(meshInfo.VertexCount, buffer);
	GTSL::Insert(meshInfo.IndexCount, buffer);
	GTSL::Insert(meshInfo.ByteOffset, buffer);
	GTSL::Insert(meshInfo.IndexSize, buffer);
}

void Extract(StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer)
{
	GTSL::Extract(meshInfo.VertexDescriptor, buffer);
	GTSL::Extract(meshInfo.VertexCount, buffer);
	GTSL::Extract(meshInfo.IndexCount, buffer);
	GTSL::Extract(meshInfo.ByteOffset, buffer);
	GTSL::Extract(meshInfo.IndexSize, buffer);
}