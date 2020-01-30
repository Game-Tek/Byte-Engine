#include <GameStudio.h>

#include <Game Studio/Utility/FlipFlop.h>
#include "Game Studio/Game/World.h"
#include "TestObject.h"
#include <Game Studio/Resources/MaterialResource.h>
#include "Game Studio/Debug/Timer.h"
#include "Game Studio/Math/GSM.hpp"
#include <Game Studio/Resources/Stream.h>
#include <Game Studio/Resources/MaterialResource.h>
#include "Game Studio/Core/System.h"

class Framebuffer;

class Sandbox final : public GS::Application
{
public:
	Sandbox() : Application(GS::ApplicationCreateInfo{"Sandbox"})
	{
		MaterialResource::MaterialData material_data;

		material_data.ResourceName = "Dou";
		
		material_data.VertexShaderCode = FString(R"(
#version 450
#extension GL_ARB_separate_shader_objects : enable

layout(push_constant) uniform INSTANCE_DATA
{
	uint instanceIndex;
} inInstanceData;

layout(binding = 0) uniform INSTANCE_TRANSFORM
{
	layout(row_major) mat4 MVP[8];
} inInstanceTransform;

layout(location = 0)in vec3 vertPos;
layout(location = 1)in vec3 vertNormal;
layout(location = 2)in vec2 vertTextureCoordinates;
layout(location = 3)in vec3 vertTangent;

layout(location = 0)out VERTEX_DATA
{
	vec4 vertPos;
	vec4 vertNormal;
	vec2 vertTexCoords;
	vec4 vertTangent;
} outVertexData;

void main()
{
	outVertexData.vertPos = inInstanceTransform.MVP[inInstanceData.instanceIndex] * vec4(vertPos, 1.0);
	outVertexData.vertTexCoords = vertTextureCoordinates;

	gl_Position = outVertexData.vertPos;
}
)");

			material_data.FragmentShaderCode = FString(R"(
#version 450

#extension GL_ARB_separate_shader_objects : enable

layout(push_constant) uniform INSTANCE_DATA
{
	uint instanceIndex;
} inInstanceData;

layout(location = 0)in VERTEX_DATA
{
	vec4 vertPos;
	vec4 vertNormal;
	vec2 vertTexCoords;
	vec4 vertTangent;
} inVertexData;

layout(location = 0) out vec4 outColor;

layout(binding = 1) uniform sampler2D textures[4096];

void main()
{
	//outColor = texture(textures[inMaterialData[instanceIndex].textureIndexes[0]], inVertexData.vertTexCoords);
	outColor = vec4(1, 1, 1, 1);
})");

		//material_data.TextureNames.resize(1);
		material_data.TextureNames.emplace_back("hydrant_Albedo");
		material_data.IsTwoSided = false;
		
		ResourceManagerInstance->CreateResource<MaterialResource>("M_Base", material_data);
		
		MyWorld = new World();
		ActiveWorld = MyWorld;

		
 		MyObject = MyWorld->CreateWorldObject<TestObject>();
		
		//auto D = Functor::MakeDelegate(&Window::GetAspectRatio, Win);
	}

	void OnUpdate() override
	{
		MyWorld->OnUpdate();
		//auto time = FString::MakeString("Time: %f", 3.14);
		//GetActiveWindow()->SetWindowTitle(time.c_str());
	}

	~Sandbox()
	{
		delete MyWorld;
		delete GetActiveWindow();
	}

	[[nodiscard]] const char* GetName() const override { return "Sandbox"; }
	const char* GetApplicationName() override { return "Sandbox"; }

	World* MyWorld = nullptr;
	TestObject* MyObject = nullptr;
	FlipFlop Flip;
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}