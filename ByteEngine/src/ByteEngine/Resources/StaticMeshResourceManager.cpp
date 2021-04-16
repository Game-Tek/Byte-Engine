#include "StaticMeshResourceManager.h"

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>
#include <assimp/scene.h>
#include <GAL/Pipelines.h>

#include <GTSL/Buffer.hpp>
#include <GAL/RenderCore.h>
#include <GTSL/Filesystem.h>
#include <GTSL/Pair.h>
#include <GTSL/Serialize.h>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Game/GameInstance.h"

using ShaderDataTypeType = GTSL::UnderlyingType<GAL::ShaderDataType>;

StaticMeshResourceManager::StaticMeshResourceManager() : ResourceManager("StaticMeshResourceManager"), meshInfos(4, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, resources_path, index_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	index_path += BE::Application::Get()->GetPathToApplication();
	query_path += "/resources/*.obj";
	index_path += "/resources/StaticMesh.beidx";
	resources_path += "/resources/";

	auto package_path = GetResourcePath(GTSL::ShortString<32>("StaticMesh"), GTSL::ShortString<32>("bepkg"));

	indexFile.OpenFile(index_path, GTSL::File::AccessMode::WRITE | GTSL::File::AccessMode::READ);
	
	if (indexFile.GetFileSize())
	{
		GTSL::Buffer<BE::TAR> meshInfosFileBuffer; meshInfosFileBuffer.Allocate(indexFile.GetFileSize(), 16, GetTransientAllocator());
		indexFile.ReadFile(meshInfosFileBuffer.GetBufferInterface());
		GTSL::Extract(meshInfos, meshInfosFileBuffer);
	}
	else
	{
		GTSL::File staticMeshPackage; staticMeshPackage.OpenFile(package_path, GTSL::File::AccessMode::WRITE);

		GTSL::FileQuery file_query(query_path);
		while(file_query.DoQuery())
		{
			auto file_path = resources_path;
			file_path += file_query.GetFileNameWithExtension();
			auto name = file_query.GetFileNameWithExtension(); name.Drop(name.FindLast('.').Get().Second);
			const auto hashed_name = GTSL::Id64(name);

			if (!meshInfos.Find(hashed_name))
			{
				GTSL::Buffer<BE::TAR> meshFileBuffer;

				GTSL::File queryFile;
				queryFile.OpenFile(file_path, GTSL::File::AccessMode::READ);
				meshFileBuffer.Allocate(queryFile.GetFileSize(), 32, GetTransientAllocator());
				queryFile.ReadFile(meshFileBuffer.GetBufferInterface());

				GTSL::Buffer<BE::TAR> meshDataBuffer; meshDataBuffer.Allocate(2048 * 2048, 8, GetTransientAllocator());

				StaticMeshDataSerialize meshInfo;

				loadMesh(meshFileBuffer, meshInfo, meshDataBuffer);

				meshInfo.ByteOffset = static_cast<uint32>(staticMeshPackage.GetFileSize());

				staticMeshPackage.WriteToFile(meshDataBuffer.GetBufferInterface());

				meshInfos.Emplace(hashed_name, meshInfo);
			}
		}

		GTSL::Buffer<BE::TAR> meshInfosFileBuffer; meshInfosFileBuffer.Allocate(4096, 16, GetTransientAllocator());
		Insert(meshInfos, meshInfosFileBuffer);

		indexFile.WriteToFile(meshInfosFileBuffer.GetBufferInterface());
	}

	initializePackageFiles(package_path);
}

StaticMeshResourceManager::~StaticMeshResourceManager()
{
}

void StaticMeshResourceManager::loadMesh(const GTSL::Buffer<BE::TAR>& sourceBuffer, StaticMeshDataSerialize& meshInfo, GTSL::Buffer<BE::TAR>& meshDataBuffer)
{
	Assimp::Importer importer;
	const auto* const ai_scene = importer.ReadFileFromMemory(sourceBuffer.GetData(), sourceBuffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
		aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_JoinIdenticalVertices);

	BE_ASSERT(ai_scene != nullptr && !(ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE), "Error interpreting file!");

	aiMesh* inMesh = ai_scene->mMeshes[0];

	struct VertexCopyData {
		const byte* Array = nullptr; uint8 ElementSize = 0, JumpSize = 0;
	};
	
	GTSL::Array<VertexCopyData, 20> vertexElements;
	
	meshInfo.VertexCount = inMesh->mNumVertices;
	
	//MESH ALWAYS HAS POSITIONS
	meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
	vertexElements.EmplaceBack(reinterpret_cast<const byte*>(inMesh->mVertices), 12, 12);

	if(inMesh->HasNormals())
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		vertexElements.EmplaceBack(reinterpret_cast<const byte*>(inMesh->mNormals), 12, 12);
	}

	if(inMesh->HasTangentsAndBitangents())
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		vertexElements.EmplaceBack(reinterpret_cast<const byte*>(inMesh->mTangents), 12, 12);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		vertexElements.EmplaceBack(reinterpret_cast<const byte*>(inMesh->mBitangents), 12, 12);
	}

	for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(inMesh->GetNumUVChannels()); ++tex_coords)
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT2);

		vertexElements.EmplaceBack(reinterpret_cast<const byte*>(inMesh->mTextureCoords[tex_coords]), 8, 12);
	}

	for (uint8 colors = 0; colors < static_cast<uint8>(inMesh->GetNumColorChannels()); ++colors)
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT4);

		vertexElements.EmplaceBack(reinterpret_cast<const byte*>(inMesh->mColors[colors]), 16, 16);
	}

	meshInfo.BoundingBox = GTSL::Vector3(); meshInfo.BoundingRadius = 0.0f;
	
	for(uint64 vertex = 0; vertex < inMesh->mNumVertices; ++vertex)
	{
		auto vertexPosition = GTSL::Vector3(inMesh->mVertices[vertex].x, inMesh->mVertices[vertex].y, inMesh->mVertices[vertex].z);
		
		meshInfo.BoundingBox = GTSL::Math::Max(meshInfo.BoundingBox, GTSL::Math::Abs(vertexPosition));

		meshInfo.BoundingRadius = GTSL::Math::Max(meshInfo.BoundingRadius, GTSL::Math::Length(vertexPosition));
		
		for(auto e : vertexElements) {
			meshDataBuffer.CopyBytes(e.ElementSize, e.Array + vertex * e.JumpSize);
		}
	}

	uint16 indexSize = 0;
	
	if((inMesh->mNumFaces * 3) < 0xFFFF)
	{
		indexSize = 2;

		for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
			for (uint32 index = 0; index < 3; ++index) {
				uint16 idx = static_cast<uint16>(inMesh->mFaces[face].mIndices[index]);
				meshDataBuffer.CopyBytes(indexSize, reinterpret_cast<byte*>(&idx));
			}
		}
	}
	else
	{
		indexSize = 4;

		for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
			for (uint32 index = 0; index < 3; ++index) {
				meshDataBuffer.CopyBytes(indexSize, reinterpret_cast<byte*>(inMesh->mFaces[face].mIndices + index));
			}
		}
	}

	meshInfo.IndexCount = inMesh->mNumFaces * 3;
	meshInfo.IndexSize = indexSize;

	meshInfo.VertexSize = GAL::GraphicsPipeline::GetVertexSize(meshInfo.VertexDescriptor);
}