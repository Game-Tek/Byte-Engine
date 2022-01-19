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

#include "GTSL/JSON.hpp"

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

	{
		GTSL::File meshDescriptionsFile; meshDescriptionsFile.Open(GetResourcePath(u8"meshes.json"));

		GTSL::Buffer buffer(GetPersistentAllocator()); meshDescriptionsFile.Read(buffer);

		GTSL::Buffer jsonBuffer(GetPersistentAllocator());

		GTSL::JSONMember json = GTSL::Parse(GTSL::StringView(buffer.GetLength(), buffer.GetLength(), reinterpret_cast<const char8_t*>(buffer.GetData())), jsonBuffer);

		for(auto mesh : json[u8"meshes"]) {			
			auto fileName = mesh[u8"name"];
			const auto hashed_name = GTSL::Id64(fileName);

			if (!resource_files_.Exists(hashed_name)) {
				GTSL::StaticString<8> fileExtension;

				GTSL::File meshFile;

				for(auto e : { u8"obj", u8"fbx" }) {
					GTSL::File queryFile;
					auto res = queryFile.Open(GetResourcePath(fileName, e), GTSL::File::READ, false);

					if(res != GTSL::File::OpenResult::ERROR) {
						fileExtension = e;
						meshFile.Open(GetResourcePath(fileName, e), GTSL::File::READ, false);
						break;
					}
				}

				GTSL::Buffer meshFileBuffer(meshFile.GetSize(), 32, GetTransientAllocator());
				meshFile.Read(meshFileBuffer);

				GTSL::Buffer meshDataBuffer(2048 * 2048, 8, GetTransientAllocator());

				StaticMeshInfo meshInfo;

				auto loadMeshSuccess = loadMesh(meshFileBuffer, meshInfo, meshDataBuffer, fileExtension);

				if (mesh[u8"meshes"].GetCount() != meshInfo.GetSubMeshes().Length) {
					BE_LOG_ERROR(u8"Incomplete data for ", mesh[u8"name"], u8" mesh.");
					continue;
				}

				for (uint32 i = 0; auto sm : mesh[u8"meshes"]) {
					auto& s = meshInfo.GetSubMeshes().array[i++];
					s.GetShaderGroupName() = sm[u8"shaderGroup"];
				}

				if (loadMeshSuccess) {
					resource_files_.AddEntry(fileName, &meshInfo, meshDataBuffer.GetRange());
				}
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

bool StaticMeshResourceManager::loadMesh(const GTSL::Buffer<BE::TAR>& sourceBuffer, StaticMeshInfo& static_mesh_data, GTSL::Buffer<BE::TAR>& meshDataBuffer, const GTSL::StringView file_extension)
{
	Assimp::Importer importer;
	const auto* const ai_scene = importer.ReadFileFromMemory(sourceBuffer.GetData(), sourceBuffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
		aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_JoinIdenticalVertices | aiProcess_MakeLeftHanded, reinterpret_cast<const char*>(file_extension.GetData()));

	if (!ai_scene || (ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE)) {
		BE_LOG_ERROR(reinterpret_cast<const char8_t*>(importer.GetErrorString()));
		return false;
	}

	if (!ai_scene->mMeshes) { return false; }

	bool interleavedStream = false;

	if (interleavedStream) {
		auto& a = static_mesh_data.GetVertexDescriptor().EmplaceBack();

		//MESH ALWAYS HAS POSITIONS
		a.EmplaceBack(GAL::ShaderDataType::FLOAT3);

		if (true) {
			a.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		}

		if (true) {
			a.EmplaceBack(GAL::ShaderDataType::FLOAT3);
			a.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		}

		for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(1); ++tex_coords) {
			a.EmplaceBack(GAL::ShaderDataType::FLOAT2);
		}

		for (uint8 colors = 0; colors < static_cast<uint8>(0); ++colors) {
			a.EmplaceBack(GAL::ShaderDataType::FLOAT4);
		}
	} else {
		//MESH ALWAYS HAS POSITIONS
		static_mesh_data.GetVertexDescriptor().EmplaceBack().EmplaceBack(GAL::ShaderDataType::FLOAT3);

		if (true) {
			static_mesh_data.GetVertexDescriptor().EmplaceBack().EmplaceBack(GAL::ShaderDataType::FLOAT3);
		}

		if (true) {
			static_mesh_data.GetVertexDescriptor().EmplaceBack().EmplaceBack(GAL::ShaderDataType::FLOAT3);
			static_mesh_data.GetVertexDescriptor().EmplaceBack().EmplaceBack(GAL::ShaderDataType::FLOAT3);
		}

		for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(1); ++tex_coords) {
			static_mesh_data.GetVertexDescriptor().EmplaceBack().EmplaceBack(GAL::ShaderDataType::FLOAT2);
		}

		for (uint8 colors = 0; colors < static_cast<uint8>(0); ++colors) {
			static_mesh_data.GetVertexDescriptor().EmplaceBack().EmplaceBack(GAL::ShaderDataType::FLOAT4);
		}
	}

	static_mesh_data.GetInterleaved() = interleavedStream;

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
		byte* const dataPointer = meshDataBuffer.GetData();
		uint64 offset = 0;

		auto visitNodeAndLoadMesh = [&](aiNode* ai_node, auto&& self) -> void {
			for (uint32 m = 0; m < ai_node->mNumMeshes; ++m) {
				const uint64 vertexSize = static_mesh_data.GetVertexSize();

				//set jump size as variable, since we can write array as interleaved or not
				uint64 jumpSize = 0;

				aiMesh* inMesh = ai_scene->mMeshes[ai_node->mMeshes[m]];

				auto matrix = ToGTSL(ai_node->mTransformation); //TODO: are mesh coordinates relative to own origin or to scene ccenter

				uint32 ai = 0, bi = 0;

				auto& meshInfo = static_mesh_data.GetSubMeshes().array[meshIndex++];

				uint64 accumulatedOffset = 0;

				auto advanceVertexElement = [&]() {
					if(interleavedStream) {
						jumpSize = vertexSize;
						auto* po = dataPointer + offset + GAL::GraphicsPipeline::GetByteOffsetToMember(bi, static_mesh_data.GetVertexDescriptor().array[ai]);
						++bi;
						return po;
					} else {
						if(bi == static_mesh_data.GetVertexDescriptor().array[ai].Length) {
							bi = 0;
							++ai;
						}

						jumpSize = GAL::GraphicsPipeline::GetByteOffsetToMember(bi + 1, static_mesh_data.GetVertexDescriptor().array[ai]);
						auto* po = dataPointer + offset + accumulatedOffset * static_mesh_data.GetVertexCount();

						accumulatedOffset += jumpSize;

						++bi;

						return po;
					}
				};

				auto writeElement = [&]<typename T>(T * elementPointer, const T & obj, const uint32 elementIndex) -> void {
					*reinterpret_cast<T*>(reinterpret_cast<byte*>(elementPointer) + (elementIndex * jumpSize)) = obj;
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
							reinterpret_cast<uint16*>(meshDataBuffer.GetData() + static_mesh_data.GetVertexCount() * vertexSize)[face * 3 + index] = static_cast<uint16>(inMesh->mFaces[face].mIndices[index]);
						}
					}
				} else {
					for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
						for (uint32 index = 0; index < 3; ++index) {
							reinterpret_cast<uint32*>(meshDataBuffer.GetData() + static_mesh_data.GetVertexCount() * vertexSize)[face * 3 + index] = static_cast<uint16>(inMesh->mFaces[face].mIndices[index]);
						}
					}
				}

				static_mesh_data.GetBoundingBox() = GTSL::Math::Max(static_mesh_data.GetBoundingBox(), meshInfo.GetBoundingBox());

				offset += meshInfo.GetVertexCount() * vertexSize + meshInfo.GetIndexCount() * static_mesh_data.GetIndexSize();
			}

			for (uint32 i = 0; i < ai_node->mNumChildren; ++i) {
				self(ai_node->mChildren[i], self);
			}
		};

		visitNodeAndLoadMesh(ai_scene->mRootNode, visitNodeAndLoadMesh);
	}

	return true;
}