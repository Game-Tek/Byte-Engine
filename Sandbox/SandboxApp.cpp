#include <GameStudio.h>

#include <Game Studio/RAPI/Window.h>
#include <Game Studio/Debug/Logger.h>

#include <Game Studio/Utility/FlipFlop.h>
#include "Game Studio/Game/World.h"
#include "TestObject.h"
#include <Game Studio/Resources/MaterialResource.h>
#include "Game Studio/Debug/Timer.h"
#include "Game Studio/Math/GSM.hpp"
#include <Game Studio/Resources/Stream.h>

class Framebuffer;

class Sandbox final : public GS::Application
{
public:
	Sandbox()
	{
		auto MatFun = [](OutStream& _OS)
		{
			FString VS("#version 450\nlayout(push_constant) uniform Push {\nmat4 Mat;\n} inPush;\nlayout(binding = 0) uniform Data {\nmat4 Pos;\n} inData;\nlayout(location = 0)in vec3 inPos;\nlayout(location = 1)in vec3 inTexCoords;\nlayout(location = 0)out vec4 tPos;\nvoid main()\n{\ntPos = inData.Pos * vec4(inPos, 1.0);\ngl_Position = tPos;\n}");

			_OS << VS;

			FString FS("#version 450\nlayout(location = 0)in vec4 tPos;\nlayout(location = 0) out vec4 outColor;\nvoid main()\n{\noutColor = vec4(tPos.x, tPos.y, tPos.z, 1);\n}");

			_OS << FS;
		};

		ResourceManagerInstance->CreateResource<MaterialResource>(FString("M_Base"), MatFun);
		
		MyWorld = new World();
		ActiveWorld = MyWorld;
		
		//GS_ASSERT(!MyWorld);

 		MyObject = MyWorld->CreateWorldObject<TestObject>(Vector3(0, 0, 25));
		
		//auto D = Functor::MakeDelegate(&Window::GetAspectRatio, Win);

		Matrix4 A(-1, 0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19);
		Matrix4 B(-2, -7, -8, -9, -10, -11, -12, -13, -14, -15, -16, -17, -18, -19, -20, -21);
		
		Matrix4 C = A * B;
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