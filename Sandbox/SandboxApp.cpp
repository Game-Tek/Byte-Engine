#include <GameStudio.h>

#include <Game Studio/Utility/FlipFlop.h>
#include "Game Studio/Game/World.h"
#include "TestObject.h"
#include <Game Studio/Resources/MaterialResource.h>
#include "Game Studio/Debug/Timer.h"
#include "Game Studio/Math/GSM.hpp"
#include <Game Studio/Resources/Stream.h>
#include <Game Studio/Resources/MaterialResource.h>
#include "Game Studio/Core/FileSystem.h"

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

layout(push_constant) uniform Push
{
	mat4 Mat;
} inPush;
layout(binding = 0) uniform Data
{
	layout(row_major) mat4 Pos;
} inData;

layout(location = 0)in vec3 inPos;
layout(location = 1)in vec3 inNormal;
layout(location = 2)in vec2 inTexCoords;

layout(location = 0)out VERTEX_DATA
{
	vec4 vertPos;
	vec4 vertNormal;
	vec2 vertTexCoords;
} outVertexData;

void main()
{
	outVertexData.vertPos = inData.Pos * vec4(inPos, 1.0);
	outVertexData.vertTexCoords = inTexCoords;

	gl_Position = outVertexData.vertPos;
}
)");

			material_data.FragmentShaderCode = FString(R"(
#version 450

#extension GL_ARB_separate_shader_objects : enable

layout(location = 0)in VERTEX_DATA
{
	vec4 vertPos;
	vec4 vertNormal;
	vec2 vertTexCoords;
} inVertexData;

layout(location = 0) out vec4 outColor;

layout(binding = 1) uniform sampler2D texSampler;

void main()
{
	outColor = texture(texSampler, inVertexData.vertTexCoords);
})");

		//material_data.TextureNames.resize(1);
		material_data.TextureNames.emplace_back(FString("hydrant_Albedo"));
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