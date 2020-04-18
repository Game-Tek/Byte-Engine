#include "ByteEngine.h"

#include "Byte Engine/Application/Application.h"
#include <iostream>

#include "Windows.h"

class Sandbox final : public BE::Application
{	
public:
	Sandbox(SystemAllocator* systemAllocator) : Application(BE::ApplicationCreateInfo{ "Sandbox", systemAllocator })
	{
		//MaterialResource::MaterialData material_data;
		//
		//material_data.ResourceName = "Dou";
		//
		//material_data.VertexShaderCode = FString(R"(
		//	#version 450
		//	#extension GL_ARB_separate_shader_objects : enable
		//	
		//	layout(push_constant) uniform INSTANCE_DATA
		//	{
		//		uint instanceIndex;
		//	} inInstanceData;
		//	
		//	layout(binding = 0) uniform INSTANCE_TRANSFORM
		//	{
		//		layout(row_major) mat4 MVP[8];
		//	} inInstanceTransform;
		//	
		//	layout(location = 0)in vec3 vertPos;
		//	layout(location = 1)in vec3 vertNormal;
		//	layout(location = 2)in vec2 vertTextureCoordinates;
		//	layout(location = 3)in vec3 vertTangent;
		//	
		//	layout(location = 0)out VERTEX_DATA
		//	{
		//		vec4 vertPos;
		//		vec4 vertNormal;
		//		vec2 vertTexCoords;
		//		vec4 vertTangent;
		//	} outVertexData;
		//	
		//	void main()
		//	{
		//		outVertexData.vertPos = inInstanceTransform.MVP[inInstanceData.instanceIndex] * vec4(vertPos, 1.0);
		//		outVertexData.vertTexCoords = vertTextureCoordinates;
		//	
		//		gl_Position = outVertexData.vertPos;
		//	}
		//	)");
		//
		//	material_data.FragmentShaderCode = FString(R"(
		//#version 450
		//
		//#extension GL_ARB_separate_shader_objects : enable
		//
		//layout(push_constant) uniform INSTANCE_DATA
		//{
		//	uint instanceIndex;
		//} inInstanceData;
		//
		//layout(location = 0)in VERTEX_DATA
		//{
		//	vec4 vertPos;
		//	vec4 vertNormal;
		//	vec2 vertTexCoords;
		//	vec4 vertTangent;
		//} inVertexData;
		//
		//layout(location = 0) out vec4 outColor;
		//
		//layout(set = 0, binding = 0) uniform sampler2D textures[4096];
		//
		//void main()
		//{
		//	//outColor = texture(textures[inMaterialData[instanceIndex].textureIndexes[0]], inVertexData.vertTexCoords);
		//	outColor = vec4(1, 1, 1, 1);
		//})");
		//
		////material_data.TextureNames.resize(1);
		//material_data.IsTwoSided = false;
		//
		//ResourceManagerInstance->CreateResource<MaterialResource>("M_Base", material_data);
		
 		//MyObject = MyWorld->CreateWorldObject<TestObject>();

#undef GetCurrentTime
		
		//BE_LOG_SUCCESS("Started at %f!", GetClock()->GetCurrentTime().Seconds<float>())
		auto text = "Hello, this is a very long string which should not fit into the first block! aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
		GTSL::String a(text, &transientAllocatorReference);

		StackAllocator::DebugData debug_data(&transientAllocatorReference);
		transientAllocator->GetDebugData(debug_data);
		printf("BytesAllocated: %llu\n", debug_data.BytesAllocated);
		printf("BytesDeallocated: %llu\n", debug_data.BytesDeallocated);
		printf("BlockMisses: %llu\n", debug_data.BlockMisses);
		printf("MemoryUsage: %llu\n", debug_data.MemoryUsage);

		transientAllocator->Clear();

		std::cout << GetName() << '\n';
	}

	void OnNormalUpdate() override
	{
	}

	void OnBackgroundUpdate() override
	{
	}
	
	~Sandbox()
	{
	}

	[[nodiscard]] const char* GetName() const override { return "Sandbox"; }
	const char* GetApplicationName() override { return "Sandbox"; }
};

BE::Application	* BE::CreateApplication(SystemAllocator* systemAllocator)
{
	return new Sandbox(systemAllocator);
}

void BE::DestroyApplication(Application* application, SystemAllocator* systemAllocator)
{
	delete application;
}
