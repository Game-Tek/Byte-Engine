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
#include <GTSL/Serialize.hpp>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Vectors.hpp>

static GTSL::Vector4 ToGTSL(const aiColor4D assimpVector) {
	return GTSL::Vector4(assimpVector.r, assimpVector.g, assimpVector.b, assimpVector.a);
}

static GTSL::Vector3 ToGTSL(const aiVector3D assimpVector) {
	return GTSL::Vector3(assimpVector.x, assimpVector.y, assimpVector.z);
}

static GTSL::Vector2 ToGTSL(const aiVector2D assimpVector) {
	return GTSL::Vector2(assimpVector.x, assimpVector.y);
}

static GTSL::Matrix4 ToGTSL(const aiMatrix4x4 assimpMatrix)
{
	return GTSL::Matrix4(
		assimpMatrix.a1, assimpMatrix.a2, assimpMatrix.a3, assimpMatrix.a4,
		assimpMatrix.b1, assimpMatrix.b2, assimpMatrix.b3, assimpMatrix.b4,
		assimpMatrix.c1, assimpMatrix.c2, assimpMatrix.c3, assimpMatrix.c4,
		assimpMatrix.d1, assimpMatrix.d2, assimpMatrix.d3, assimpMatrix.d4
	);
}

using ShaderDataTypeType = GTSL::UnderlyingType<GAL::ShaderDataType>;

StaticMeshResourceManager::StaticMeshResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"StaticMeshResourceManager")
{
	GTSL::StaticString<512> query_path, resources_path, index_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	query_path += u8"/resources/*.obj";
	resources_path += u8"/resources/StaticMesh";

	resource_files_.Start(resources_path);

	GTSL::FileQuery file_query;
	while(auto queryResult = file_query.DoQuery(query_path))
	{
		auto file_path = resources_path;
		file_path += queryResult.Get();
		auto fileName = queryResult.Get(); DropLast(fileName, u8'.');
		const auto hashed_name = GTSL::Id64(fileName);

		if (!resource_files_.Exists(hashed_name)) {
			GTSL::File queryFile;
			queryFile.Open(GetResourcePath(queryResult.Get()), GTSL::File::READ, false);
			
			GTSL::Buffer meshFileBuffer(queryFile.GetSize(), 32, GetTransientAllocator());
			queryFile.Read(meshFileBuffer);

			GTSL::Buffer meshDataBuffer(2048 * 2048, 8, GetTransientAllocator());

			StaticMeshInfo meshInfo;

			if (loadMesh(meshFileBuffer, meshInfo, meshDataBuffer)) {
				resource_files_.AddEntry(fileName, &meshInfo, meshDataBuffer.GetRange());
			}
		}
	}
}

StaticMeshResourceManager::~StaticMeshResourceManager()
{
}

template<typename T, uint8 N>
struct nEl {
	T e[N];
};

bool StaticMeshResourceManager::loadMesh(const GTSL::Buffer<BE::TAR>& sourceBuffer, StaticMeshInfo& static_mesh_data, GTSL::Buffer<BE::TAR>& meshDataBuffer)
{
	Assimp::Importer importer;
	const auto* const ai_scene = importer.ReadFileFromMemory(sourceBuffer.GetData(), sourceBuffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
		aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_JoinIdenticalVertices | aiProcess_MakeLeftHanded | aiProcess_FlipWindingOrder, "obj");

	if (!ai_scene || (ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE)) {
		BE_LOG_ERROR(reinterpret_cast<const char8_t*>(importer.GetErrorString()));
		return false;
	}

	if (!ai_scene->mMeshes) { return false; }

	//MESH ALWAYS HAS POSITIONS
	static_mesh_data.GetVertexDescriptor().EmplaceBack(GAL::ShaderDataType::FLOAT3);

	if (true) {
		static_mesh_data.GetVertexDescriptor().EmplaceBack(GAL::ShaderDataType::FLOAT3);
	}

	if (true) {
		static_mesh_data.GetVertexDescriptor().EmplaceBack(GAL::ShaderDataType::FLOAT3);
		static_mesh_data.GetVertexDescriptor().EmplaceBack(GAL::ShaderDataType::FLOAT3);
	}

	for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(1); ++tex_coords) {
		static_mesh_data.GetVertexDescriptor().EmplaceBack(GAL::ShaderDataType::FLOAT2);
	}

	for (uint8 colors = 0; colors < static_cast<uint8>(0); ++colors) {
		static_mesh_data.GetVertexDescriptor().EmplaceBack(GAL::ShaderDataType::FLOAT4);
	}

	static_mesh_data.GetVertexCount() = 0; static_mesh_data.GetIndexCount() = 0; static_mesh_data.GetBoundingRadius() = 0; static_mesh_data.GetBoundingBox() = GTSL::Vector3();

	{
		static_mesh_data.GetIndexSize() = 0;

		auto visitNodeValidateAndAllocate = [&](aiNode* ai_node, auto&& self) -> bool {
			for (uint32 i = 0; i < ai_node->mNumMeshes; ++i) {
				auto inMesh = ai_scene->mMeshes[ai_node->mMeshes[i]];
				if (!(inMesh->mPrimitiveTypes & aiPrimitiveType_TRIANGLE)) { return false; }

				if (!inMesh->HasNormals()) { return false; }
				if (!inMesh->HasTangentsAndBitangents()) { return false; }
				if (!inMesh->HasTextureCoords(0)) { return false; }
				if (inMesh->HasVertexColors(0)) { return false; }

				auto& meshInfo = static_mesh_data.GetSubMeshes().EmplaceBack();

				meshInfo.GetVertexCount() = inMesh->mNumVertices;
				meshInfo.GetIndexCount() = inMesh->mNumFaces * 3;
				meshInfo.GetBoundingBox() = GTSL::Vector3(); meshInfo.GetBoundingRadius() = 0.0f;
				meshInfo.GetMaterialIndex() = inMesh->mMaterialIndex;

				static_mesh_data.GetIndexCount() += meshInfo.GetIndexCount();
				static_mesh_data.GetVertexCount() += meshInfo.GetVertexCount();

				if(inMesh->mNumVertices > 0xFFFF) {
					GTSL::Max(&static_mesh_data.GetIndexSize(), static_cast<uint8>(4));
				} else {
					GTSL::Max(&static_mesh_data.GetIndexSize(), static_cast<uint8>(2));
				}
			}

			for(uint32 i = 0; i < ai_node->mNumChildren; ++i) {
				self(ai_node->mChildren[i], self);
			}

			return true;
		};

		if(!visitNodeValidateAndAllocate(ai_scene->mRootNode, visitNodeValidateAndAllocate)) { return false; }

		//meshDataBuffer.Resize(static_mesh_data.GetVertexCount() * static_mesh_data.GetVertexSize() + static_mesh_data.GetIndexCount() * static_mesh_data.GetIndexSize());
		meshDataBuffer.AddBytes(static_mesh_data.GetVertexCount() * static_mesh_data.GetVertexSize() + static_mesh_data.GetIndexCount() * static_mesh_data.GetIndexSize());
	}

	{
		uint32 meshIndex = 0;
		byte* dataPointer = meshDataBuffer.GetData();

		auto visitNodeAndLoadMesh = [&](aiNode* ai_node, auto&& self) -> void {
			for (uint32 m = 0; m < ai_node->mNumMeshes; ++m) {
				aiMesh* inMesh = ai_scene->mMeshes[ai_node->mMeshes[m]];

				auto matrix = ToGTSL(ai_node->mTransformation); //TODO: are mesh coordinates relative to own origin or to scene ccenter

				uint32 elementIndex = 0;

				auto& meshInfo = static_mesh_data.GetSubMeshes().array[meshIndex++];

				auto advanceVertexElement = [&]() {
					return dataPointer + GAL::GraphicsPipeline::GetByteOffsetToMember(elementIndex++, static_mesh_data.GetVertexDescriptor());
				};

				auto writeElement = [&]<typename T>(T * elementPointer, const T & obj, const uint32 elementIndex) -> void {
					*reinterpret_cast<T*>(reinterpret_cast<byte*>(elementPointer) + (elementIndex * static_mesh_data.GetVertexSize())) = obj;
				};

				auto doWrite = [writeElement]<typename T>(const GAL::ShaderDataType format, T value, byte * byteData, uint32 vertexIndex) {
					switch (format) {
					case GAL::ShaderDataType::FLOAT2: {
						if constexpr (std::is_same_v<GTSL::Vector2, T>) {
							auto* positions = reinterpret_cast<GTSL::Vector2*>(byteData);
							writeElement(positions, value, vertexIndex);
						}

						if constexpr (std::is_same_v<GTSL::Vector3, T>) {
							auto* positions = reinterpret_cast<GTSL::Vector2*>(byteData);
							writeElement(positions, GTSL::Vector2(value[0], value[1]), vertexIndex);
						}

						break;
					}
					case GAL::ShaderDataType::FLOAT3: {
						if constexpr (std::is_same_v<GTSL::Vector3, T>) {
							auto* positions = reinterpret_cast<GTSL::Vector3*>(byteData);
							writeElement(positions, value, vertexIndex);
						}
						break;
					}
					case GAL::ShaderDataType::U16_SNORM3: {
						if constexpr (std::is_same_v<GTSL::Vector3, T>) {
							auto* positions = reinterpret_cast<nEl<int16, 3>*>(byteData);
							writeElement(positions, { GAL::FloatToSNORM(value[0]), GAL::FloatToSNORM(value[1]), GAL::FloatToSNORM(value[2]) }, vertexIndex);
						}

						break;
					}
					case GAL::ShaderDataType::U16_UNORM2: {
						if constexpr (std::is_same_v<GTSL::Vector3, T>) {
							auto* positions = reinterpret_cast<nEl<uint16, 2>*>(byteData);
							auto textureCoordinate = GTSL::Vector2(value);
							writeElement(positions, { GAL::FloatToUNORM(textureCoordinate[0]), GAL::FloatToUNORM(textureCoordinate[1]) }, vertexIndex);
						}
						break;
					}
					case GAL::ShaderDataType::UINT64: {
						if constexpr (std::is_same_v<GTSL::Vector3, T>) {
							auto* positions = reinterpret_cast<nEl<int8, 3>*>(byteData);
							writeElement(positions, { static_cast<int8>(value[0] * static_cast<int8>(127)), static_cast<int8>(value[1] * static_cast<int8>(127)), static_cast<int8>(value[2] * static_cast<int8>(127)) }, vertexIndex);
						}
						break;
					}
					case GAL::ShaderDataType::FLOAT4: {
						if constexpr (std::is_same_v<GTSL::Vector4, T>) {
							auto* positions = reinterpret_cast<GTSL::Vector4*>(byteData);
							writeElement(positions, value, vertexIndex);
						}
						break;
					}
					}
				};

				auto writeAndReadVertexComponent = [&]<typename T>(GAL::ShaderDataType format, T * assimpDataArray, auto && perVertexFunc) {
					auto* byteData = advanceVertexElement();

					for (uint64 vertexIndex = 0; vertexIndex < inMesh->mNumVertices; ++vertexIndex) {
						auto assimpAttribute = ToGTSL(assimpDataArray[vertexIndex]);
						perVertexFunc(assimpAttribute);
						doWrite(format, assimpAttribute, byteData, vertexIndex);
					}
				};

				auto writeVertexComponent = [&]<typename T>(GAL::ShaderDataType format, T * assimpDataArray) -> void {
					auto* byteData = advanceVertexElement();

					for (uint64 vertexIndex = 0; vertexIndex < inMesh->mNumVertices; ++vertexIndex) {
						doWrite(format, ToGTSL(assimpDataArray[vertexIndex]), byteData, vertexIndex);
					}
				};

				writeAndReadVertexComponent(GAL::ShaderDataType::FLOAT3, inMesh->mVertices, [&](GTSL::Vector3 position) {
					meshInfo.GetBoundingBox() = GTSL::Math::Max(meshInfo.GetBoundingBox(), GTSL::Math::Abs(position));
					meshInfo.GetBoundingRadius() = GTSL::Math::Max(meshInfo.GetBoundingRadius(), GTSL::Math::Length(position));
					});

				if (inMesh->HasNormals()) {
					writeVertexComponent(GAL::ShaderDataType::FLOAT3, inMesh->mNormals);
				}

				if (inMesh->HasTangentsAndBitangents()) {
					writeVertexComponent(GAL::ShaderDataType::FLOAT3, inMesh->mTangents);
					writeVertexComponent(GAL::ShaderDataType::FLOAT3, inMesh->mBitangents);
				}

				for (uint8 i = 0; i < 8 && inMesh->HasTextureCoords(i); ++i) {
					writeVertexComponent(GAL::ShaderDataType::FLOAT2, inMesh->mTextureCoords[i]);
				}

				for (uint8 i = 0; i < 8 && inMesh->HasVertexColors(i); ++i) {
					writeVertexComponent(GAL::ShaderDataType::FLOAT4, inMesh->mColors[i]);
				}

				//write all vertices together at start, all indices together at the end
				//mesh[0].Vertices[], mesh[1].Vertices[], mesh[0].Indices[], mesh[1].Indices[]
				if (static_mesh_data.GetIndexSize() == 2) {
					//if (inMesh->mNumFaces * 3 < 0xFF) {
					//	indexSize = 1;
					//
					//	for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
					//		for (uint32 index = 0; index < 3; ++index) {
					//			uint8 idx = static_cast<uint8>(inMesh->mFaces[face].mIndices[index]);
					//			meshDataBuffer.CopyBytes(indexSize, reinterpret_cast<byte*>(&idx));
					//		}
					//	}
					//}
					//else {

					for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
						for (uint32 index = 0; index < 3; ++index) {
							reinterpret_cast<uint16*>(meshDataBuffer.GetData() + static_mesh_data.GetVertexCount() * static_mesh_data.GetVertexSize())[face * 3 + index] = static_cast<uint16>(inMesh->mFaces[face].mIndices[index]);
						}
					}
				} else {
					for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
						for (uint32 index = 0; index < 3; ++index) {
							reinterpret_cast<uint32*>(meshDataBuffer.GetData() + static_mesh_data.GetVertexCount() * static_mesh_data.GetVertexSize())[face * 3 + index] = static_cast<uint16>(inMesh->mFaces[face].mIndices[index]);
						}
					}
				}

				static_mesh_data.GetBoundingBox() = GTSL::Math::Max(static_mesh_data.GetBoundingBox(), meshInfo.GetBoundingBox());

				dataPointer += meshInfo.GetVertexCount() * static_mesh_data.GetVertexSize() + meshInfo.GetIndexCount() * static_mesh_data.GetIndexSize();
			}

			for (uint32 i = 0; i < ai_node->mNumChildren; ++i) {
				self(ai_node->mChildren[i], self);
			}
		};

		visitNodeAndLoadMesh(ai_scene->mRootNode, visitNodeAndLoadMesh);
	}

	return true;
}