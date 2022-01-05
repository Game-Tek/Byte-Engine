#pragma once

#include <GAL/Pipelines.h>
#include <GAL/RenderCore.h>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/String.hpp>
#include <GTSL/File.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/Serialize.hpp>
#include <GTSL/Math/Vectors.hpp>

#include <GAL/Serialize.hpp>

#include "ResourceManager.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Render/ShaderGenerator.h"
#include "GTSL/Filesystem.h"

template<typename T, class A>
auto operator<<(auto& buffer, const GTSL::Vector<T, A>& vector) -> decltype(buffer)& {
	buffer << vector.GetLength();
	for (uint32 i = 0; i < vector.GetLength(); ++i) { buffer << vector[i]; }
	return buffer;
}

template<typename T, class A>
auto operator>>(auto& buffer, GTSL::Vector<T, A>& vector) -> decltype(buffer)& {
	uint32 length;
	buffer >> length;
	for (uint32 i = 0; i < length; ++i) { buffer >> vector.EmplaceBack(); }
	return buffer;
}

template<class A>
auto operator<<(auto& buffer, const GTSL::String<A>& vector) -> decltype(buffer)& {
	buffer << vector.GetBytes() << vector.GetCodepoints();
	for (uint32 i = 0; i < vector.GetBytes(); ++i) { buffer << vector.c_str()[i]; }
	return buffer;
}

template<class A>
auto operator>>(auto& buffer, GTSL::String<A>& vector) -> decltype(buffer)& {
	uint32 length, codepoints;
	buffer >> length >> codepoints;
	for (uint32 i = 0; i < length; ++i) {
		char8_t c;
		buffer >> c;
		vector += c;
	}
	return buffer;
}

template<typename T, class A>
auto Read(auto& buffer, GTSL::Vector<T, A>& vector, const BE::PAR& allocator) -> decltype(buffer)& {
	uint32 length;
	buffer >> length;
	for (uint32 i = 0; i < length; ++i) { Extract(vector.EmplaceBack(), buffer); }
	return buffer;
}

template<uint8 S>
auto operator<<(auto& buffer, const GTSL::ShortString<S>& string) -> decltype(buffer)& {
	for (uint32 i = 0; i < S; ++i) { buffer << string.begin()[i]; }
	return buffer;
}

template<uint8 S>
auto operator>>(auto& buffer, GTSL::ShortString<S>& string) -> decltype(buffer)& {
	for (uint32 i = 0; i < S; ++i) { buffer >> const_cast<char8_t*>(string.begin())[i]; }
	return buffer;
}

template<GTSL::Enum E>
auto operator<<(auto& buffer, const E enu) -> decltype(buffer)& {
	buffer << static_cast<GTSL::UnderlyingType<E>>(enu);
	return buffer;
}

template<GTSL::Enum E>
auto operator>>(auto& buffer, E& enu) -> decltype(buffer)& {
	buffer >> reinterpret_cast<GTSL::UnderlyingType<E>&>(enu);
	return buffer;
}

static unsigned long long quickhash64(const GTSL::Range<const byte*> range) { // set 'mix' to some value other than zero if you want a tagged hash          
	const unsigned long long mulp = 2654435789;
	unsigned long long mix = 0;

	mix ^= 104395301;

	for (auto e : range)
		mix += (e * mulp) ^ (mix >> 23);

	return mix ^ (mix << 37);
}

class ShaderResourceManager final : public ResourceManager
{
	static GTSL::ShortString<12> ShaderTypeToFileExtension(GAL::ShaderType type) {
		switch (type) {
		case GAL::ShaderType::VERTEX: return u8"vert";
		case GAL::ShaderType::TESSELLATION_CONTROL: return u8"tesc";
		case GAL::ShaderType::TESSELLATION_EVALUATION: return u8"tese";
		case GAL::ShaderType::GEOMETRY: return u8"geom";
		case GAL::ShaderType::FRAGMENT: return u8"frag";
		case GAL::ShaderType::COMPUTE: return u8"comp";
		case GAL::ShaderType::RAY_GEN: return u8"rgen";
		case GAL::ShaderType::ANY_HIT: return u8"rahit";
		case GAL::ShaderType::CLOSEST_HIT: return u8"rchit";
		case GAL::ShaderType::MISS: return u8"rmiss";
		case GAL::ShaderType::INTERSECTION: return u8"rint";
		case GAL::ShaderType::CALLABLE: return u8"rcall";
		}
	}

public:
	static StructElement readStructElement(GTSL::JSONMember json) {
		return { json[u8"type"], json[u8"name"], json[u8"defaultValue"] };
	}

	using ShaderHash = uint64;

	ShaderResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"ShaderResourceManager"), shaderGroupInfoOffsets(8, GetPersistentAllocator()), shaderInfoOffsets(8, GetPersistentAllocator()), shaderOffsets(8, GetPersistentAllocator()) {
		shaderPackageFile.Open(GetResourcePath(u8"Shaders", u8"bepkg"), GTSL::File::READ | GTSL::File::WRITE, true);

		GTSL::File shaderGroupsTableFile, shaderInfoTableFile, shadersTableFile;
		shaderGroupsTableFile.Open(GetResourcePath(u8"ShaderGroups.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);
		shaderInfoTableFile.Open(GetResourcePath(u8"ShaderInfo.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);
		shadersTableFile.Open(GetResourcePath(u8"Shaders.betbl"), GTSL::File::READ | GTSL::File::WRITE, true);

		bool created = false;

		switch (shaderInfosFile.Open(GetResourcePath(u8"Shaders", u8"beidx"), GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		}

		switch (shaderGroupInfosFile.Open(GetResourcePath(u8"ShaderGroups", u8"beidx"), GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		}

		if (!(shaderPackageFile.GetSize() && shaderGroupsTableFile.GetSize() && shaderInfoTableFile.GetSize() && shadersTableFile.GetSize() && shaderInfosFile.GetSize() && shaderGroupInfosFile.GetSize())) {
			shaderPackageFile.Resize(0);
			shaderGroupsTableFile.Resize(0);
			shaderInfoTableFile.Resize(0);
			shadersTableFile.Resize(0);
			shaderInfosFile.Resize(0);
			shaderGroupInfosFile.Resize(0);
			created = true;
		}

		if (created) {
			GTSL::KeyMap<ShaderHash, BE::TAR> loadedShaders(128, GetTransientAllocator());

			GTSL::FileQuery shaderGroupFileQuery;

			while (auto fileRef = shaderGroupFileQuery.DoQuery(GetResourcePath(u8"*ShaderGroup.json"))) {
				GTSL::File shaderGroupFile; shaderGroupFile.Open(GetResourcePath(fileRef.Get()), GTSL::File::READ, false);

				GTSL::Buffer buffer(shaderGroupFile.GetSize(), 16, GetTransientAllocator()); shaderGroupFile.Read(buffer);

				GTSL::Buffer deserializer(GetTransientAllocator());
				auto json = Parse(GTSL::StringView(GTSL::Byte(buffer.GetLength()), reinterpret_cast<const utf8*>(buffer.GetData())), deserializer);

				ShaderGroupDataSerialize shaderGroupDataSerialize(GetPersistentAllocator());
				shaderGroupDataSerialize.Name = json[u8"name"];

				GPipeline pipeline = makeDefaultPipeline();

				auto rasterModelHandle = pipeline.Add(GPipeline::ElementHandle(), u8"rasterModel", GPipeline::LanguageElement::ElementType::MODEL);
				auto rayTraceModelHandle = pipeline.Add(GPipeline::ElementHandle(), u8"rayTraceModel", GPipeline::LanguageElement::ElementType::MODEL);

				GPipeline::ElementHandle vertexShaderScope, fragmentShaderScope, computeShaderScope, rayGenShaderScope, closestHitShaderScope, missShaderScope;

				vertexShaderScope = pipeline.Add(GPipeline::ElementHandle(), u8"VertexShader", GPipeline::LanguageElement::ElementType::SCOPE);
				fragmentShaderScope = pipeline.Add(GPipeline::ElementHandle(), u8"FragmentShader", GPipeline::LanguageElement::ElementType::SCOPE);
				computeShaderScope = pipeline.Add(GPipeline::ElementHandle(), u8"ComputeShader", GPipeline::LanguageElement::ElementType::SCOPE);
				rayGenShaderScope = pipeline.Add(GPipeline::ElementHandle(), u8"RayGenShader", GPipeline::LanguageElement::ElementType::SCOPE);
				closestHitShaderScope = pipeline.Add(GPipeline::ElementHandle(), u8"ClosestHitShader", GPipeline::LanguageElement::ElementType::SCOPE);
				missShaderScope = pipeline.Add(GPipeline::ElementHandle(), u8"MissShader", GPipeline::LanguageElement::ElementType::SCOPE);

				if (auto jsonVertex = json[u8"vertexElements"]) {
					GTSL::StaticVector<StructElement, 8> vertexElements;

					for (auto ve : jsonVertex) {
						shaderGroupDataSerialize.VertexElements.EmplaceBack(ve[u8"type"], ve[u8"id"]);
						vertexElements.EmplaceBack(ve[u8"type"], ve[u8"id"]);
					}

					pipeline.DeclareStruct(GPipeline::ElementHandle(), u8"vertex", vertexElements);
					pipeline.DeclareStruct(GPipeline::ElementHandle(), u8"index", { { u8"u16vec3", u8"indexTri" } });
				}

				pipeline.DeclareStruct(GPipeline::ElementHandle(), u8"globalData", { { u8"uint32", u8"frameIndex" }, {u8"float32", u8"time"} });
				pipeline.DeclareStruct(GPipeline::ElementHandle(), u8"cameraData", { { u8"mat4f", u8"view" }, {u8"mat4f", u8"proj"}, {u8"mat4f", u8"viewInverse"}, {u8"mat4f", u8"projInverse"} });
				pipeline.DeclareStruct(GPipeline::ElementHandle(), u8"renderPassData", { { u8"ImageReference", u8"Color" }, {u8"ImageReference", u8"Normal" }, { u8"ImageReference", u8"Depth"} });

				for (auto e : { vertexShaderScope, fragmentShaderScope, closestHitShaderScope }) {
					auto instanceDataStruct = pipeline.Add(e, u8"instanceData", GPipeline::LanguageElement::ElementType::STRUCT);
					pipeline.DeclareVariable(instanceDataStruct, { u8"mat4f", u8"ModelMatrix" });
					auto instanceStructVertexBuffer = pipeline.DeclareVariable(instanceDataStruct, { u8"vertex*", u8"VertexBuffer" });
					auto instanceStructIndexBuffer = pipeline.DeclareVariable(instanceDataStruct, { u8"index*", u8"IndexBuffer" });
					pipeline.DeclareVariable(instanceDataStruct, { u8"uint32", u8"MaterialInstance" });
				}

				auto rasterPushConstantBlockHandle = pipeline.Add(rasterModelHandle, u8"pushConstantBlock", GPipeline::LanguageElement::ElementType::MEMBER);
				pipeline.DeclareVariable(rasterPushConstantBlockHandle, { u8"globalData*", u8"global" });
				pipeline.DeclareVariable(rasterPushConstantBlockHandle, { u8"cameraData*", u8"camera" });
				pipeline.DeclareVariable(rasterPushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
				auto rasterPushConstantShaderParameters = pipeline.DeclareVariable(rasterPushConstantBlockHandle, { u8"shaderParametersData*", u8"shaderParameters" });
				pipeline.DeclareVariable(rasterPushConstantBlockHandle, { u8"instanceData*", u8"instance" });

				pipeline.DeclareVariable(fragmentShaderScope, { u8"vec4f", u8"Color" });
				pipeline.DeclareVariable(fragmentShaderScope, { u8"vec4f", u8"Normal" });

				auto vertexSurfaceInterface = pipeline.Add(rasterModelHandle, u8"vertexSurfaceInterface", GPipeline::LanguageElement::ElementType::SCOPE);
				auto vertexTextureCoordinatesHandle = pipeline.DeclareVariable(vertexSurfaceInterface, { u8"vec2f", u8"vertexTextureCoordinates" });
				pipeline.AddMemberDeductionGuide(vertexShaderScope, u8"vertexTextureCoordinates", { { vertexSurfaceInterface }, { vertexTextureCoordinatesHandle } });
				auto vertexViewSpacePositionHandle = pipeline.DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"viewSpacePosition" });
				pipeline.AddMemberDeductionGuide(vertexShaderScope, u8"vertexViewSpacePosition", { { vertexSurfaceInterface }, { vertexViewSpacePositionHandle } });
				auto vertexViewSpaceNormalHandle = pipeline.DeclareVariable(vertexSurfaceInterface, { u8"vec3f", u8"viewSpaceNormal" });
				pipeline.AddMemberDeductionGuide(vertexShaderScope, u8"vertexViewSpaceNormal", { { vertexSurfaceInterface }, { vertexViewSpaceNormalHandle } });
				auto glPositionHandle = pipeline.DeclareVariable(vertexShaderScope, { u8"vec4f", u8"gl_Position" });
				pipeline.AddMemberDeductionGuide(vertexShaderScope, u8"vertexPosition", { glPositionHandle });

				pipeline.DeclareStruct(rayGenShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });
				pipeline.DeclareStruct(missShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });
				pipeline.DeclareStruct(closestHitShaderScope, u8"traceRayParameterData", { { u8"uint64", u8"AccelerationStructure"}, {u8"uint32", u8"RayFlags"}, {u8"uint32", u8"SBTRecordOffset"}, {u8"uint32", u8"SBTRecordStride"}, {u8"uint32", u8"MissIndex"}, {u8"float32", u8"tMin"}, {u8"float32", u8"tMax"} });

				pipeline.DeclareStruct(rayGenShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"uint64", u8"instances" } });
				pipeline.DeclareStruct(missShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"uint64", u8"instances" } });
				pipeline.DeclareStruct(closestHitShaderScope, u8"rayTraceData", { { u8"traceRayParameterData", u8"traceRayParameters"}, { u8"instanceData*", u8"instances" } });

				auto rayTracePushConstantBlockHandle = pipeline.Add(rayTraceModelHandle, u8"pushConstantBlock", GPipeline::LanguageElement::ElementType::MEMBER);
				pipeline.DeclareVariable(rayTracePushConstantBlockHandle, { u8"globalData*", u8"global" });
				pipeline.DeclareVariable(rayTracePushConstantBlockHandle, { u8"cameraData*", u8"camera" });
				pipeline.DeclareVariable(rayTracePushConstantBlockHandle, { u8"renderPassData*", u8"renderPass" });
				pipeline.DeclareVariable(rayTracePushConstantBlockHandle, { u8"rayTraceData*", u8"rayTrace" });

				auto payloadBlockHandle = pipeline.Add(rayTraceModelHandle, u8"payloadBlock", GPipeline::LanguageElement::ElementType::SCOPE);
				auto payloadHandle = pipeline.DeclareVariable(payloadBlockHandle, { u8"vec4f", u8"payload" });

				pipeline.DeclareRawFunction(fragmentShaderScope, u8"vec2f", u8"GetFragmentPosition", {}, u8"return gl_FragCoord.xy;");
				pipeline.DeclareRawFunction(fragmentShaderScope, u8"float32", u8"GetFragmentDepth", {}, u8"return gl_FragCoord.z;");
				pipeline.DeclareRawFunction(fragmentShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"return vertexIn.vertexTextureCoordinates;");
				pipeline.DeclareRawFunction(fragmentShaderScope, u8"mat4f", u8"GetInverseProjectionMatrix", {}, u8"return pushConstantBlock.camera.projInverse;");
				pipeline.DeclareRawFunction(fragmentShaderScope, u8"vec3f", u8"GetVertexViewSpacePosition", {}, u8"return vertexIn.viewSpacePosition;");
				pipeline.DeclareRawFunction(fragmentShaderScope, u8"vec4f", u8"GetSurfaceViewSpaceNormal", {}, u8"return vec4(vertexIn.viewSpaceNormal, 0);");
				auto fragmentOutputBlockHandle = pipeline.Add(fragmentShaderScope, u8"fragmentOutputBlock", GPipeline::LanguageElement::ElementType::MEMBER);
				auto outColorHandle = pipeline.DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Color" });
				auto outNormalHandle = pipeline.DeclareVariable(fragmentOutputBlockHandle, { u8"vec4f", u8"out_Normal" });
				pipeline.AddMemberDeductionGuide(fragmentShaderScope, u8"surfaceColor", { outColorHandle });
				pipeline.AddMemberDeductionGuide(fragmentShaderScope, u8"surfaceNormal", { outNormalHandle });

				pipeline.DeclareRawFunction(vertexShaderScope, u8"vec4f", u8"GetVertexPosition", {}, u8"return vec4(POSITION, 1);");
				pipeline.DeclareRawFunction(vertexShaderScope, u8"vec4f", u8"GetVertexNormal", {}, u8"return vec4(NORMAL, 0);");
				pipeline.DeclareRawFunction(vertexShaderScope, u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"return TEXTURE_COORDINATES;");
				pipeline.DeclareRawFunction(vertexShaderScope, u8"mat4f", u8"GetInstancePosition", {}, u8"return pushConstantBlock.instance.ModelMatrix;");
				pipeline.DeclareRawFunction(vertexShaderScope, u8"mat4f", u8"GetCameraViewMatrix", {}, u8"return pushConstantBlock.camera.view;");
				pipeline.DeclareRawFunction(vertexShaderScope, u8"mat4f", u8"GetCameraProjectionMatrix", {}, u8"return pushConstantBlock.camera.proj;");

				pipeline.DeclareRawFunction(computeShaderScope, u8"uvec2", u8"GetScreenPosition", {}, u8"return gl_WorkGroupID.xy;");

				pipeline.DeclareRawFunction(rayGenShaderScope, u8"mat4f", u8"GetInverseViewMatrix", {}, u8"return pushConstantBlock.camera.viewInverse;");
				pipeline.DeclareRawFunction(rayGenShaderScope, u8"mat4f", u8"GetInverseProjectionMatrix", {}, u8"return pushConstantBlock.camera.projInverse;");
				pipeline.DeclareRawFunction(rayGenShaderScope, u8"void", u8"TraceRay", { { u8"vec4f", u8"origin" }, { u8"vec4f", u8"direction" } }, u8"traceRayParameterData r = pushConstantBlock.rayTrace.traceRayParameters; traceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, vec3f(origin), r.tMin, vec3f(direction), r.tMax, 0);");
				pipeline.DeclareRawFunction(rayGenShaderScope, u8"vec2u", u8"GetFragmentPosition", {}, u8" return gl_LaunchIDEXT.xy;");
				pipeline.DeclareRawFunction(rayGenShaderScope, u8"vec2f", u8"GetFragmentNormalizedPosition", {}, u8"vec2f pixelCenter = vec2f(gl_LaunchIDEXT.xy) + vec2f(0.5f); return pixelCenter / vec2f(gl_LaunchSizeEXT.xy - 1);");

				pipeline.DeclareVariable(closestHitShaderScope, { u8"vec2f", u8"hitBarycenter" });
				auto shaderRecordBlockHandle = pipeline.Add(closestHitShaderScope, u8"shaderRecordBlock", GPipeline::LanguageElement::ElementType::MEMBER);
				auto shaderRecordEntry = pipeline.DeclareVariable(shaderRecordBlockHandle, { u8"shaderParametersData*", u8"shaderEntries" });
				pipeline.AddMemberDeductionGuide(closestHitShaderScope, u8"surfaceColor", { payloadHandle });
				pipeline.Add(closestHitShaderScope, u8"surfaceNormal", GPipeline::LanguageElement::ElementType::DISABLED);
				pipeline.DeclareFunction(closestHitShaderScope, u8"vec3f", u8"GetVertexBarycenter", {}, u8"return Barycenter(hitBarycenter);");
				pipeline.DeclareFunction(closestHitShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"instanceData* instance = pushConstantBlock.rayTrace.instances[gl_InstanceCustomIndexEXT]; u16vec3 indices = instance.IndexBuffer[gl_PrimitiveID].indexTri; vec3f barycenter = GetVertexBarycenter(); return instance.VertexBuffer[indices[0]].TEXTURE_COORDINATES * barycenter.x + instance.VertexBuffer[indices[1]].TEXTURE_COORDINATES * barycenter.y + instance.VertexBuffer[indices[2]].TEXTURE_COORDINATES * barycenter.z;");
				//pipeline.DeclareFunction(closestHitShaderScope, u8"vec2f", u8"GetSurfaceTextureCoordinates", {}, u8"instanceData* instance = pushConstantBlock.rayTrace.instances[0]; u16vec3 indices = instance.IndexBuffer.indexTri[gl_PrimitiveID]; vec3f barycenter = GetVertexBarycenter(); return instance.VertexBuffer[indices[0]].TEXTURE_COORDINATES * 0.33f + instance.VertexBuffer[indices[1]].TEXTURE_COORDINATES * 0.33f + instance.VertexBuffer[indices[2]].TEXTURE_COORDINATES * 0.33f;");

				GTSL::StaticVector<uint64, 16> shaderGroupUsedShaders;

				for (auto s : json[u8"structs"]) {
					GTSL::StaticVector<StructElement, 8> elements;

					for (auto m : s[u8"members"]) {
						elements.EmplaceBack(m[u8"type"], m[u8"name"]);
					}

					pipeline.DeclareStruct(GPipeline::ElementHandle(), s[u8"name"], elements);
				}

				for (auto f : json[u8"functions"]) {
					GTSL::StaticVector<StructElement, 8> elements;
					for (auto p : f[u8"params"]) { elements.EmplaceBack(p[u8"type"], p[u8"name"]); }
					pipeline.DeclareFunction(GPipeline::ElementHandle(), f[u8"type"], f[u8"name"], elements, f[u8"code"]);
				}

				for (auto i : json[u8"instances"]) {
					auto& instance = shaderGroupDataSerialize.Instances.EmplaceBack();
					instance.Name = i[u8"name"];

					for (auto f : i[u8"parameters"]) {
						auto& param = instance.Parameters.EmplaceBack();
						param.First = f[u8"name"];
						param.Second = f[u8"defaultValue"];
					}
				}

				for (auto p : json[u8"parameters"]) {
					shaderGroupDataSerialize.Parameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);
				}

				auto processShaderGroup = [&](GTSL::JSONMember json, ShaderGroupDataSerialize& shaderGroupDataSerialize, bool rayTrace) {
					GTSL::StaticVector<StructElement, 8> shaderParameters;

					for (auto p : json[u8"parameters"]) {
						shaderParameters.EmplaceBack(p[u8"type"], p[u8"name"], p[u8"defaultValue"]);
					}

					for (auto s : json[u8"shaders"]) {
						GTSL::File shaderFile; shaderFile.Open(GetResourcePath(s[u8"name"], u8"json"));
						GTSL::Buffer shaderFileBuffer(shaderFile.GetSize(), 16, GetTransientAllocator()); shaderFile.Read(shaderFileBuffer);

						auto shader = makeShader(shaderFileBuffer, pipeline);

						GPipeline::ElementHandle shaderSemanticsScope;
						GAL::ShaderType targetSemantics;

						switch (shader.Type) {
						case Shader::Class::VERTEX:
							if (rayTrace) {
								targetSemantics = GAL::ShaderType::COMPUTE; shaderSemanticsScope = computeShaderScope;
								continue;
							}
							else {
								targetSemantics = GAL::ShaderType::VERTEX; shaderSemanticsScope = vertexShaderScope;
							}
							break;
						case Shader::Class::SURFACE:
							if (rayTrace) {
								targetSemantics = GAL::ShaderType::CLOSEST_HIT; shaderSemanticsScope = closestHitShaderScope;
							}
							else {
								targetSemantics = GAL::ShaderType::FRAGMENT; shaderSemanticsScope = fragmentShaderScope;
							}
							break;
						case Shader::Class::COMPUTE: targetSemantics = GAL::ShaderType::COMPUTE; shaderSemanticsScope = computeShaderScope; break;
						case Shader::Class::RENDER_PASS: break;
						case Shader::Class::RAY_GEN:
							if (rayTrace) {
								targetSemantics = GAL::ShaderType::RAY_GEN; shaderSemanticsScope = rayGenShaderScope;
							}
							else {
								continue;
							}
							break;
						case Shader::Class::MISS:
							if (rayTrace) {
								targetSemantics = GAL::ShaderType::MISS; shaderSemanticsScope = missShaderScope;
							}
							else {
								continue;
								targetSemantics = GAL::ShaderType::COMPUTE; shaderSemanticsScope = computeShaderScope;
							}
							break;
						}

						auto shaderScope = pipeline.Add(shaderSemanticsScope, shader.Name, GPipeline::LanguageElement::ElementType::SCOPE);

						{
							auto shaderParametersHandle = pipeline.DeclareStruct(shaderScope, u8"shaderParametersData", shaderParameters);

							for (auto& e : shaderParameters) {
								if (rayTrace) {
									pipeline.AddMemberDeductionGuide(shaderScope, shaderParameters.back().Name, { shaderRecordEntry, pipeline.GetElementHandle(shaderParametersHandle, e.Name) });
								}
								else {
									pipeline.AddMemberDeductionGuide(shaderScope, shaderParameters.back().Name, { rasterPushConstantBlockHandle, rasterPushConstantShaderParameters, pipeline.GetElementHandle(shaderParametersHandle, e.Name) });
								}
							}
						}

						pipeline.DeclareFunction(shaderScope, u8"void", u8"main"); //main

						auto shaderPermutationScopes = std::initializer_list<const GPipeline::ElementHandle>{ {}, shaderSemanticsScope, rayTrace ? rayTraceModelHandle : rasterModelHandle, shaderScope };

						evalShader(shaderFileBuffer, shader, pipeline, shaderPermutationScopes);
						auto shaderResult = GenerateShader(shader, pipeline, shaderPermutationScopes, targetSemantics);
						if (!shaderResult) { BE_LOG_WARNING(shaderResult.Get().Second); }
						auto shaderHash = quickhash64(GTSL::Range(shaderResult.Get().First.GetBytes(), reinterpret_cast<const byte*>(shaderResult.Get().First.c_str())));

						if (loadedShaders.Find(shaderHash)) { continue; }
						loadedShaders.Emplace(shaderHash);

						auto [compRes, resultString, shaderBuffer] = CompileShader(shaderResult.Get().First, s[u8"name"], targetSemantics, GAL::ShaderLanguage::GLSL, GetTransientAllocator());

						if (!compRes) { BE_LOG_ERROR(shaderResult.Get().First); BE_LOG_ERROR(resultString); }

						shaderInfoTableFile << shaderHash << shaderInfosFile.GetSize(); //shader info table
						shadersTableFile << shaderHash << shaderPackageFile.GetSize(); //shader table

						shaderInfosFile << GTSL::ShortString<32>(s[u8"name"]) << static_cast<uint32>(shaderBuffer.GetLength()) << shaderHash;
						shaderInfosFile << 0; //0 params
						shaderInfosFile << targetSemantics;

						shaderPackageFile.Write(shaderBuffer);

						shaderGroupDataSerialize.Shaders.EmplaceBack(shaderGroupUsedShaders.GetLength());
						shaderGroupUsedShaders.EmplaceBack(shaderHash);
					}
				};

				bool rayTrace = true; ShaderGroupInfo::RayTraceData ray_trace_data;

				if (rayTrace) {
					processShaderGroup(json, shaderGroupDataSerialize, false);
					processShaderGroup(json, shaderGroupDataSerialize, true);
				}
				else {
					processShaderGroup(json, shaderGroupDataSerialize, false);
				}

				shaderGroupsTableFile << shaderGroupDataSerialize.Name << shaderGroupInfosFile.GetSize();

				{
					shaderGroupInfosFile << shaderGroupDataSerialize.Name;

					shaderGroupInfosFile << shaderGroupUsedShaders.GetLength();
					for (auto& e : shaderGroupUsedShaders) { shaderGroupInfosFile << e; }

					shaderGroupInfosFile << shaderGroupDataSerialize.Parameters.GetLength();
					for (auto& p : shaderGroupDataSerialize.Parameters) {
						shaderGroupInfosFile << p.Type << p.Name << p.Value;
					}

					shaderGroupInfosFile << shaderGroupDataSerialize.Instances.GetLength();
					for (auto& i : shaderGroupDataSerialize.Instances) {
						shaderGroupInfosFile << i.Name;

						shaderGroupInfosFile << i.Parameters.GetLength();
						for (auto& p : i.Parameters) {
							shaderGroupInfosFile << p.First << p.Second;
						}
					}

					shaderGroupInfosFile << shaderGroupDataSerialize.VertexElements.GetLength();

					for (auto& e : shaderGroupDataSerialize.VertexElements) {
						shaderGroupInfosFile << e.Type << e.Name;
					}

					shaderGroupInfosFile << rayTrace;

					if (rayTrace) {
						shaderGroupInfosFile << ray_trace_data.Payload.Type << ray_trace_data.Payload.Name << ray_trace_data.Payload.DefaultValue;

						for (uint32 i = 0; i < 4; ++i) {
							shaderGroupInfosFile << ray_trace_data.Groups[i].ShadersPerGroup.GetLength();

							for (uint32 j = 0; j < ray_trace_data.Groups[i].ShadersPerGroup.GetLength(); ++j) {
								shaderGroupInfosFile << ray_trace_data.Groups[i].ShadersPerGroup[j];
							}
						}
					}
				}
			}
		}

		shaderGroupsTableFile.SetPointer(0);
		{
			uint32 offset = 0;
			while (offset < shaderGroupsTableFile.GetSize()) {
				GTSL::ShortString<32> name; uint64 position;
				shaderGroupsTableFile >> name >> position;
				offset += 32 + 8;
				shaderGroupInfoOffsets.Emplace(Id(name), position);
			}
		}

		shaderInfoTableFile.SetPointer(0);
		{
			uint32 offset = 0;
			while (offset < shaderInfoTableFile.GetSize()) {
				uint64 name; uint64 position;
				shaderInfoTableFile >> name >> position;
				offset += 8 + 8;
				shaderInfoOffsets.Emplace(name, position);
			}
		}

		shadersTableFile.SetPointer(0);
		{
			uint32 offset = 0;
			while (offset < shadersTableFile.GetSize()) {
				uint64 name; uint64 position;
				shadersTableFile >> name >> position;
				offset += 8 + 8;
				shaderOffsets.Emplace(name, position);
			}
		}
	}

	~ShaderResourceManager() = default;

	struct Parameter {
		GTSL::StaticString<32> Type, Name, Value;

		Parameter() = default;
		Parameter(const GTSL::StringView type, const GTSL::StringView name, const GTSL::StringView val) : Type(type), Name(name), Value(val) {}

		template<class ALLOC>
		friend void Insert(const Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer) {
			Insert(parameterInfo.Type, buffer);
			Insert(parameterInfo.Name, buffer);
			Insert(parameterInfo.Value, buffer);
		}

		template<class ALLOC>
		friend void Extract(Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer) {
			Extract(parameterInfo.Type, buffer);
			Extract(parameterInfo.Name, buffer);
			Extract(parameterInfo.Value, buffer);
		}
	};

	struct ShaderGroupInstance {
		ShaderGroupInstance() = default;

		GTSL::ShortString<32> Name;
		GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 16> Parameters;

		ShaderGroupInstance& operator=(const ShaderGroupInstance& shader_group_instance) {
			Name = shader_group_instance.Name; Parameters = shader_group_instance.Parameters;
			return *this;
		}

		template<class ALLOC>
		friend void Insert(const ShaderGroupInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer) {
			Insert(materialInstance.Name, buffer);
			Insert(materialInstance.Parameters, buffer);
		}

		template<class ALLOC>
		friend void Extract(ShaderGroupInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer) {
			Extract(materialInstance.Name, buffer);
			Extract(materialInstance.Parameters, buffer);
		}

	};

	struct VertexShader {};

	struct FragmentShader {
		GAL::BlendOperation WriteOperation;

		template<class ALLOC>
		friend void Insert(const FragmentShader& fragment_shader, GTSL::Buffer<ALLOC>& buffer) {
			Insert(fragment_shader.WriteOperation, buffer);
		}

		template<class ALLOC>
		friend void Extract(FragmentShader& fragment_shader, GTSL::Buffer<ALLOC>& buffer) {
			Extract(fragment_shader.WriteOperation, buffer);
		}
	};

	struct TaskShader {

	};

	struct MeshShader {

	};

	struct ComputeShader {

	};

	struct RayGenShader {
		uint8 Recursion = 1;
	};

	struct ClosestHitShader {

	};

	struct MissShader {

	};

	struct AnyHitShader {

	};

	struct IntersectionShader {

	};

	struct CallableShader {

	};

	struct ShaderInfo {
		GTSL::ShortString<32> Name;
		GAL::ShaderType Type; uint64 Hash = 0;
		GTSL::StaticVector<Parameter, 8> Parameters;
		uint32 Size = 0;

		union {
			VertexShader VertexShader;
			FragmentShader FragmentShader;
			ComputeShader ComputeShader;
			TaskShader TaskShader;
			MeshShader MeshShader;
			RayGenShader RayGenShader;
			ClosestHitShader ClosestHitShader;
			MissShader MissShader;
			AnyHitShader AnyHitShader;
			IntersectionShader IntersectionShader;
			CallableShader CallableShader;
		};

		ShaderInfo() {}

		void SetType(GAL::ShaderType type) {
			Type = type;

			switch (Type) {
			case GAL::ShaderType::VERTEX: ::new(&VertexShader) struct VertexShader(); break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: break;
			case GAL::ShaderType::COMPUTE: ::new(&ComputeShader) struct ComputeShader(); break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: ::new(&RayGenShader) struct RayGenShader(); break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			default: __debugbreak();
			}
		}

		ShaderInfo(const ShaderInfo& shader_info) : Name(shader_info.Name), Type(shader_info.Type), Hash(shader_info.Hash), Parameters(shader_info.Parameters), Size(shader_info.Size) {
			switch (Type) {
			case GAL::ShaderType::VERTEX: ::new(&VertexShader) struct VertexShader(shader_info.VertexShader); break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: ::new(&FragmentShader) struct FragmentShader(shader_info.FragmentShader); break;
			case GAL::ShaderType::COMPUTE: ::new(&ComputeShader) struct ComputeShader(shader_info.ComputeShader); break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: ::new(&RayGenShader) struct RayGenShader(shader_info.RayGenShader); break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			}
		}

		~ShaderInfo() {
			switch (Type) {
			case GAL::ShaderType::VERTEX: GTSL::Destroy(VertexShader); break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: GTSL::Destroy(FragmentShader); break;
			case GAL::ShaderType::COMPUTE: GTSL::Destroy(ComputeShader); break;
			case GAL::ShaderType::TASK: GTSL::Destroy(TaskShader); break;
			case GAL::ShaderType::MESH: GTSL::Destroy(MeshShader); break;
			case GAL::ShaderType::RAY_GEN: GTSL::Destroy(RayGenShader); break;
			case GAL::ShaderType::ANY_HIT: GTSL::Destroy(AnyHitShader); break;
			case GAL::ShaderType::CLOSEST_HIT: GTSL::Destroy(ClosestHitShader); break;
			case GAL::ShaderType::MISS: GTSL::Destroy(MissShader); break;
			case GAL::ShaderType::INTERSECTION: GTSL::Destroy(IntersectionShader); break;
			case GAL::ShaderType::CALLABLE: GTSL::Destroy(CallableShader); break;
			default:;
			}
		}

		ShaderInfo& operator=(const ShaderInfo& other) {
			Size = other.Size;
			Name = other.Name;
			Type = other.Type;
			Hash = other.Hash;
			Parameters = other.Parameters;

			switch (Type) {
			case GAL::ShaderType::VERTEX: VertexShader = other.VertexShader; break;
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: FragmentShader = other.FragmentShader; break;
			case GAL::ShaderType::COMPUTE: ComputeShader = other.ComputeShader; break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: RayGenShader = other.RayGenShader; break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			}

			return *this;
		}

		template<class ALLOC>
		friend void Insert(const ShaderInfo& shader, GTSL::Buffer<ALLOC>& buffer) {
			Insert(shader.Name, buffer);
			Insert(shader.Type, buffer);
			Insert(shader.Size, buffer);
			Insert(shader.Parameters, buffer);

			switch (shader.Type) {
			case GAL::ShaderType::VERTEX: Insert(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Insert(shader.FragmentShader, buffer); break;
			case GAL::ShaderType::COMPUTE: Insert(shader.ComputeShader, buffer); break;
			}
		}

		template<class ALLOC>
		friend void Extract(ShaderInfo& shader, GTSL::Buffer<ALLOC>& buffer) {
			Extract(shader.Name, buffer);
			Extract(shader.Type, buffer);
			Extract(shader.Size, buffer);
			Extract(shader.Parameters, buffer);

			switch (shader.Type) {
			case GAL::ShaderType::VERTEX: Extract(shader.VertexShader, buffer); break;
			case GAL::ShaderType::FRAGMENT: Extract(shader.FragmentShader, buffer); break;
			case GAL::ShaderType::COMPUTE: Extract(shader.ComputeShader, buffer); break;
			}
		}
	};

	struct ShaderGroupData : Data {
		ShaderGroupData(const BE::PAR& allocator) : Parameters(allocator), Instances(allocator), Shaders(allocator) {}

		GTSL::ShortString<32> Name;

		GTSL::Vector<Parameter, BE::PAR> Parameters;
		GTSL::Vector<ShaderGroupInstance, BE::PAR> Instances;
		GTSL::Vector<uint32, BE::PAR> Shaders;
		GTSL::StaticVector<StructElement, 20> VertexElements;
	};

	struct ShaderGroupDataSerialize : ShaderGroupData, Object {
		ShaderGroupDataSerialize(const BE::PAR& allocator) : ShaderGroupData(allocator) {}
	};

	struct ShaderGroupInfo {
		ShaderGroupInfo(const BE::PAR& allocator) : Shaders(allocator), Instances(allocator), Parameters(allocator) {}

		GTSL::ShortString<32> Name;

		GTSL::Vector<ShaderInfo, BE::PAR> Shaders;
		GTSL::Vector<ShaderGroupInstance, BE::PAR> Instances;
		GTSL::Vector<Parameter, BE::PAR> Parameters;

		GTSL::StaticVector<StructElement, 16> VertexElements;

		struct RayTraceData {
			StructElement Payload;

			struct Group {
				GTSL::StaticVector<uint32, 8> ShadersPerGroup;
			} Groups[4];
		} RayTrace;
	};

	template<typename... ARGS>
	void LoadShaderGroupInfo(ApplicationManager* gameInstance, Id shaderGroupName, DynamicTaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"loadShaderInfosFromDisk", {}, &ShaderResourceManager::loadShaderGroup<ARGS...>, {}, {}, GTSL::MoveRef(shaderGroupName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadShaderGroup(ApplicationManager* gameInstance, ShaderGroupInfo shader_group_info, DynamicTaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, GTSL::Range<byte*> buffer, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"loadShadersFromDisk", {}, &ShaderResourceManager::loadShaders<ARGS...>, {}, {}, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File shaderGroupInfosFile, shaderInfosFile, shaderPackageFile;
	GTSL::HashMap<Id, uint64, BE::PersistentAllocatorReference> shaderGroupInfoOffsets;
	GTSL::HashMap<uint64, uint64, BE::PersistentAllocatorReference> shaderInfoOffsets, shaderOffsets;
	mutable GTSL::ReadWriteMutex mutex;

	template<typename... ARGS>
	void loadShaderGroup(TaskInfo taskInfo, Id shaderGroupName, DynamicTaskHandle<ShaderGroupInfo, ARGS...> dynamicTaskHandle, ARGS... args) { //TODO: check why can't use ARGS&&
		shaderGroupInfosFile.SetPointer(shaderGroupInfoOffsets[shaderGroupName]);

		ShaderGroupInfo shaderGroupInfo(GetPersistentAllocator());

		shaderGroupInfosFile >> shaderGroupInfo.Name;

		uint32 shaderCount;
		shaderGroupInfosFile >> shaderCount;

		for (uint32 s = 0; s < shaderCount; ++s) {
			uint64 shaderHash;
			shaderGroupInfosFile >> shaderHash;

			auto& shader = shaderGroupInfo.Shaders.EmplaceBack();

			{
				shaderInfosFile.SetPointer(shaderInfoOffsets[shaderHash]);
				shaderInfosFile >> shader.Name >> shader.Size >> shader.Hash;

				uint32 paramCount = 0;
				shaderInfosFile >> paramCount;

				for (uint32 p = 0; p < paramCount; ++p) {
					auto& parameter = shader.Parameters.EmplaceBack();
					shaderInfosFile >> parameter.Name >> parameter.Type >> parameter.Value;
				}

				GAL::ShaderType shaderType;
				shaderInfosFile >> shaderType;

				shader.SetType(shaderType);
			}
		}

		uint32 parameterCount;
		shaderGroupInfosFile >> parameterCount;

		for (uint32 p = 0; p < parameterCount; ++p) {
			auto& parameter = shaderGroupInfo.Parameters.EmplaceBack();
			shaderGroupInfosFile >> parameter.Type >> parameter.Name >> parameter.Value;
		}

		uint32 instanceCount;
		shaderGroupInfosFile >> instanceCount;

		for (uint32 i = 0; i < instanceCount; ++i) {
			auto& instance = shaderGroupInfo.Instances.EmplaceBack();
			shaderGroupInfosFile >> instance.Name;

			uint32 params = 0;
			shaderGroupInfosFile >> params;

			for (uint32 p = 0; p < params; ++p) {
				auto& param = instance.Parameters.EmplaceBack();
				shaderGroupInfosFile >> param.First >> param.Second;
			}
		}

		uint32 vertexElementCount = 0;
		shaderGroupInfosFile >> vertexElementCount;

		for (uint32 i = 0; i < vertexElementCount; ++i) {
			auto& vertexElement = shaderGroupInfo.VertexElements.EmplaceBack();
			shaderGroupInfosFile >> vertexElement.Type >> vertexElement.Name;
		}

		bool rayTrace = false; shaderGroupInfosFile >> rayTrace;

		if (rayTrace) {
			shaderGroupInfosFile >> shaderGroupInfo.RayTrace.Payload.Type >> shaderGroupInfo.RayTrace.Payload.Name >> shaderGroupInfo.RayTrace.Payload.DefaultValue;

			for (uint32 i = 0; i < 4; ++i) {
				uint32 groupCount; shaderGroupInfosFile >> groupCount;

				for (uint32 j = 0; j < groupCount; ++j) {
					shaderGroupInfosFile >> shaderGroupInfo.RayTrace.Groups[i].ShadersPerGroup.EmplaceBack();
				}
			}
		}

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(shaderGroupInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadShaders(TaskInfo taskInfo, ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer, DynamicTaskHandle<ShaderGroupInfo, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, ARGS... args) {
		uint32 offset = 0;

		for (const auto& s : shader_group_info.Shaders) {
			shaderPackageFile.SetPointer(shaderOffsets[s.Hash]);
			shaderPackageFile.Read(s.Size, offset, buffer);
			offset += s.Size;
		}

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(shader_group_info), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
	};

	GPipeline makeDefaultPipeline() {
		GPipeline pipeline;
		auto descriptorSetBlockHandle = pipeline.Add(GPipeline::ElementHandle(), u8"descriptorSetBlock", GPipeline::LanguageElement::ElementType::SCOPE);
		auto firstDescriptorSetBlockHandle = pipeline.Add(descriptorSetBlockHandle, u8"descriptorSet", GPipeline::LanguageElement::ElementType::SCOPE);
		pipeline.DeclareVariable(firstDescriptorSetBlockHandle, { u8"texture2D[]", u8"textures" });
		pipeline.DeclareVariable(firstDescriptorSetBlockHandle, { u8"image2D[]", u8"images" });
		pipeline.DeclareVariable(firstDescriptorSetBlockHandle, { u8"sampler", u8"s" });

		pipeline.DeclareStruct({}, u8"TextureReference", { { u8"uint32", u8"Instance" } });
		pipeline.DeclareStruct({}, u8"ImageReference", { { u8"uint32", u8"Instance" } });

		pipeline.DeclareRawFunction({}, u8"vec3f", u8"Barycenter", { { u8"vec2f", u8"coords" } }, u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);");
		pipeline.DeclareRawFunction({}, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" } }, u8"return texture(sampler2D(textures[nonuniformEXT(tex.Instance)], s), texCoord);");
		pipeline.DeclareRawFunction({}, u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"uvec2", u8"pos" } }, u8"return texelFetch(sampler2D(textures[nonuniformEXT(tex.Instance)], s), ivec2(pos), 0);");
		pipeline.DeclareRawFunction({}, u8"vec4f", u8"Sample", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" } }, u8"return imageLoad(images[nonuniformEXT(img.Instance)], ivec2(pos));");
		pipeline.DeclareRawFunction({}, u8"void", u8"Write", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" }, { u8"vec4f", u8"value" } }, u8"imageStore(images[nonuniformEXT(img.Instance)], ivec2(pos), value);");
		pipeline.DeclareRawFunction({}, u8"float32", u8"X", { { u8"vec4f", u8"vec" } }, u8"return vec.x;");
		pipeline.DeclareRawFunction({}, u8"float32", u8"Y", { { u8"vec4f", u8"vec" } }, u8"return vec.y;");
		pipeline.DeclareRawFunction({}, u8"float32", u8"Z", { { u8"vec4f", u8"vec" } }, u8"return vec.z;");
		pipeline.DeclareRawFunction({}, u8"vec3f", u8"FresnelSchlick", { { u8"float32", u8"cosTheta" }, { u8"vec3f", u8"F0" } }, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
		pipeline.DeclareRawFunction({}, u8"vec3f", u8"Normalize", { { u8"vec3f", u8"a" } }, u8"return normalize(a);");
		pipeline.DeclareRawFunction({}, u8"float32", u8"Sigmoid", { { u8"float32", u8"x" } }, u8"return 1.0 / (1.0 + pow(x / (1.0 - x), -3.0));");
		pipeline.DeclareRawFunction({}, u8"vec3f", u8"WorldPositionFromDepth", { { u8"vec2f", u8"texture_coordinate" }, { u8"float32", u8"depth_from_depth_buffer" }, { u8"mat4f", u8"inverse_projection_matrix" } }, u8"vec4 p = inverse_projection_matrix * vec4(vec3(texture_coordinate * 2.0 - vec2(1.0), depth_from_depth_buffer), 1.0); return p.xyz / p.w;\n");

		return pipeline;
	}

	Shader makeShader(const GTSL::Buffer<BE::TAR>& shaderFileBuffer, GPipeline& pipeline) {
		GTSL::Buffer json_deserializer(BE::TAR(u8"GenerateShader"));
		auto shaderJson = Parse(GTSL::StringView(GTSL::Byte(shaderFileBuffer.GetLength()), reinterpret_cast<const utf8*>(shaderFileBuffer.GetData())), json_deserializer);

		Shader::Class shaderClass;

		switch (Hash(shaderJson[u8"class"])) {
		case GTSL::Hash(u8"Vertex"): shaderClass = Shader::Class::VERTEX; break;
		case GTSL::Hash(u8"Surface"): shaderClass = Shader::Class::SURFACE; break;
		case GTSL::Hash(u8"Compute"): shaderClass = Shader::Class::COMPUTE; break;
		case GTSL::Hash(u8"RayGen"): shaderClass = Shader::Class::RAY_GEN; break;
		case GTSL::Hash(u8"Miss"): shaderClass = Shader::Class::MISS; break;
		}

		Shader shader(shaderJson[u8"name"], shaderClass);

		if (shaderClass == ::Shader::Class::COMPUTE) {
			if (auto res = shaderJson[u8"localSize"]) {
				shader.SetThreadSize({ static_cast<uint16>(res[0].GetUint()), static_cast<uint16>(res[1].GetUint()), static_cast<uint16>(res[2].GetUint()) });
			}
			else {
				shader.SetThreadSize({ 1, 1, 1 });
			}
		}

		if (auto sv = shaderJson[u8"shaderVariables"]) {
			for (auto e : sv) {
				StructElement struct_element(e[u8"type"], e[u8"name"]);

				pipeline.Add(GPipeline::ElementHandle(), struct_element.Name, GPipeline::LanguageElement::ElementType::MEMBER);

				if (auto res = e[u8"defaultValue"]) {
					struct_element.DefaultValue = res;
				}

				shader.ShaderParameters.EmplaceBack(struct_element);
			}
		}

		if (auto tr = shaderJson[u8"transparency"]) {
			shader.Transparency = tr.GetBool();
		}

		return shader;
	}

	void evalShader(const GTSL::Buffer<BE::TAR>& shaderFileBuffer, Shader& shader, GPipeline& pipeline, const GTSL::Range<const GPipeline::ElementHandle*> scopes) {
		GTSL::Buffer json_deserializer(BE::TAR(u8"GenerateShader"));
		auto shaderJson = Parse(GTSL::StringView(GTSL::Byte(shaderFileBuffer.GetLength()), reinterpret_cast<const utf8*>(shaderFileBuffer.GetData())), json_deserializer);

		if (auto fs = shaderJson[u8"functions"]) {
			for (auto f : fs) {
				auto& fd = shader.Functions.EmplaceBack();

				fd.Return = f[u8"return"];
				fd.Name = f[u8"name"];

				pipeline.Add(GPipeline::ElementHandle(), fd.Name, GPipeline::LanguageElement::ElementType::FUNCTION);

				for (auto p : f[u8"params"]) { fd.Parameters.EmplaceBack(p[u8"type"], p[u8"name"]); }

				parseCode(f[u8"code"].GetStringView(), pipeline, fd.Statements, scopes);
			}
		}

		if (auto code = shaderJson[u8"code"]) {
			parseCode(code.GetStringView(), pipeline, pipeline.GetFunction(scopes, u8"main").Statements, scopes);
		}
	}
};
