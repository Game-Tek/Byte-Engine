#include "GameApplication.h"

#include <iostream>

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Debug/FunctionTimer.h"
#include "ByteEngine/Game/CameraSystem.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Render/LightsRenderGroup.h"
#include "ByteEngine/Render/MaterialSystem.h"
#include "ByteEngine/Render/RenderOrchestrator.h"
#include "ByteEngine/Render/StaticMeshRenderGroup.h"

#include "ByteEngine/Render/RenderSystem.h"
#include "ByteEngine/Render/UIManager.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"
#include "ByteEngine/Resources/PipelineCacheResourceManager.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

#include "ByteEngine/Resources/AudioResourceManager.h"
#include "ByteEngine/Resources/FontResourceManager.h"
#include "ByteEngine/Sound/AudioSystem.h"

#include <GTSL/GamepadQuery.h>

#pragma comment(lib, "XInput.lib")

class RenderOrchestrator;

bool GameApplication::Initialize()
{
	if(!Application::Initialize()) { return false; } 
	
	SetupInputSources();
	
	CreateResourceManager<StaticMeshResourceManager>();
	CreateResourceManager<TextureResourceManager>();
	CreateResourceManager<MaterialResourceManager>();
	CreateResourceManager<AudioResourceManager>();
	CreateResourceManager<PipelineCacheResourceManager>();
	//CreateResourceManager<FontResourceManager>();

	return true;
}

void GameApplication::PostInitialize()
{
	//FRAME START
	gameInstance->AddStage("FrameStart");

	//GAMEPLAY CODE BEGINS
	gameInstance->AddStage("GameplayStart");
	//GAMEPLAY CODE ENDS
	gameInstance->AddStage("GameplayEnd");
	
	//RENDER CODE BEGINS
	gameInstance->AddStage("RenderStart");
	//RENDER SETUP BEGINS
	gameInstance->AddStage("RenderStartSetup");
	//RENDER SETUP ENDS
	gameInstance->AddStage("RenderEndSetup");
	//RENDER IS DISPATCHED
	gameInstance->AddStage("RenderDo");
	//RENDER DISPATCH IS DONE
	gameInstance->AddStage("RenderFinished");
	//RENDER CODE ENDS
	gameInstance->AddStage("RenderEnd");
	
	//FRAME ENDS
	gameInstance->AddStage("FrameEnd");

	gameInstance->AddEvent("Application", EventHandle<>("OnFocusGain"));
	gameInstance->AddEvent("Application", EventHandle<>("OnFocusLoss"));
	
	auto* renderSystem = gameInstance->AddSystem<RenderSystem>("RenderSystem");
	auto* materialSystem = gameInstance->AddSystem<MaterialSystem>("MaterialSystem");
	auto* renderOrchestrator = gameInstance->AddSystem<RenderOrchestrator>("RenderOrchestrator");

	gameInstance->AddSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	gameInstance->AddSystem<AudioSystem>("AudioSystem");

	GTSL::Window::WindowCreateInfo create_window_info;
	create_window_info.Application = &systemApplication;
	create_window_info.Name = GTSL::StaticString<1024>(GetApplicationName());
	create_window_info.Extent = { 1280, 720 };
	create_window_info.Type = GTSL::Window::WindowType::OS_WINDOW;
	create_window_info.UserData = this;
	create_window_info.Function = GTSL::Delegate<void(void*, GTSL::Window::WindowEvents, void*)>::Create<GameApplication, &GameApplication::windowUpdateFunction>(this);
	window.BindToOS(create_window_info); //Call bind to OS after declaring goals, RenderSystem and RenderOrchestrator; as window creation may call ResizeDelegate which
	//queues a function that depends on these elements existing

	window.AddDevice(GTSL::Window::DeviceType::MOUSE);
	
	renderSystem->SetWindow(&window);

	window.ShowWindow();
	
	gameInstance->AddSystem<CameraSystem>("CameraSystem");
	
	{
		renderOrchestrator->AddAttachment("Color", 8, 4, GAL::ComponentType::INT, TextureType::COLOR, GTSL::RGBA(0, 0, 0, 0));
		renderOrchestrator->AddAttachment("Position", 16, 4, GAL::ComponentType::FLOAT, TextureType::COLOR, GTSL::RGBA(0, 0, 0, 0));
		renderOrchestrator->AddAttachment("Normal", 16, 4, GAL::ComponentType::FLOAT, TextureType::COLOR, GTSL::RGBA(0, 0, 0, 0));
		renderOrchestrator->AddAttachment("RenderDepth", 32, 1, GAL::ComponentType::FLOAT, TextureType::DEPTH, GTSL::RGBA(1.0f, 0, 0, 0));

		GTSL::Array<RenderOrchestrator::PassData, 6> passes;
		RenderOrchestrator::PassData geoRenderPass;
		geoRenderPass.Name = "SceneRenderPass";
		geoRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		geoRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ "Color" } );
		geoRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ "Position" } );
		geoRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ "Normal" } );
		geoRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ "RenderDepth" } );
		geoRenderPass.ResultAttachment = "Color";
		passes.EmplaceBack(geoRenderPass);

		RenderOrchestrator::PassData uiRenderPass{};
		uiRenderPass.Name = "UIRenderPass";
		uiRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		uiRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ "Color" });
		uiRenderPass.ResultAttachment = "Color";
		passes.EmplaceBack(uiRenderPass);

		RenderOrchestrator::PassData rtRenderPass{};
		rtRenderPass.Name = "SceneRTRenderPass";
		rtRenderPass.PassType = RenderOrchestrator::PassType::RAY_TRACING;
		rtRenderPass.ReadAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ "Position" });
		rtRenderPass.ReadAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ "Normal" });
		rtRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ "Color" });
		rtRenderPass.ResultAttachment = "Color";
		passes.EmplaceBack(rtRenderPass);
		
		renderOrchestrator->AddPass(renderSystem, materialSystem, passes);
		renderOrchestrator->ToggleRenderPass("SceneRenderPass", true);
		renderOrchestrator->ToggleRenderPass("UIRenderPass", false);
		renderOrchestrator->ToggleRenderPass("SceneRTRenderPass", true);
	}

	
	auto* uiManager = gameInstance->AddSystem<UIManager>("UIManager");
	gameInstance->AddSystem<CanvasSystem>("CanvasSystem");
	
	gameInstance->AddSystem<StaticMeshRenderManager>("StaticMeshRenderManager");
	gameInstance->AddSystem<UIRenderManager>("UIRenderManager");
	gameInstance->AddSystem<LightsRenderGroup>("LightsRenderGroup");
	
	renderOrchestrator->AddRenderManager(gameInstance, "StaticMeshRenderManager", gameInstance->GetSystemReference("StaticMeshRenderManager"));
	renderOrchestrator->AddRenderManager(gameInstance, "UIRenderManager", gameInstance->GetSystemReference("UIRenderManager"));
}	

void GameApplication::OnUpdate(const OnUpdateInfo& updateInfo)
{
	Application::OnUpdate(updateInfo);

	PROFILE;

	window.Update(this, GTSL::Delegate<void(void*, GTSL::Window::WindowEvents, void*)>::Create<GameApplication, &GameApplication::windowUpdateFunction>(this));

	auto button = [&](GTSL::Gamepad::GamepadButtonPosition button, bool state)
	{
		switch (button)
		{
		case GTSL::Gamepad::GamepadButtonPosition::TOP: GetInputManager()->RecordActionInputSource("Controller", controller, "TopFrontButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::RIGHT: GetInputManager()->RecordActionInputSource("Controller", controller, "RightFrontButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::BOTTOM: GetInputManager()->RecordActionInputSource("Controller", controller, "BottomFrontButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::LEFT: GetInputManager()->RecordActionInputSource("Controller", controller, "LeftFrontButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::BACK: GetInputManager()->RecordActionInputSource("Controller", controller, "LeftMenuButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::HOME: GetInputManager()->RecordActionInputSource("Controller", controller, "RightMenuButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::DPAD_UP: GetInputManager()->RecordActionInputSource("Controller", controller, "TopDPadButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::DPAD_RIGHT: GetInputManager()->RecordActionInputSource("Controller", controller, "RightDPadButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::DPAD_DOWN: GetInputManager()->RecordActionInputSource("Controller", controller, "BottomDPadButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::DPAD_LEFT: GetInputManager()->RecordActionInputSource("Controller", controller, "LeftDPadButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::LEFT_SHOULDER: GetInputManager()->RecordActionInputSource("Controller", controller, "LeftHatButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::RIGHT_SHOULDER: GetInputManager()->RecordActionInputSource("Controller", controller, "RightHatButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::LEFT_STICK: GetInputManager()->RecordActionInputSource("Controller", controller, "LeftStickButton", state); break;
		case GTSL::Gamepad::GamepadButtonPosition::RIGHT_STICK: GetInputManager()->RecordActionInputSource("Controller", controller, "RightStickButton", state); break;
		default: ;
		}
	};

	auto floats = [&](GTSL::Gamepad::Side side, const float32 value)
	{
		switch (side)
		{
		case GTSL::Gamepad::Side::RIGHT: Get()->GetInputManager()->RecordLinearInputSource("Controller", controller, "RightTrigger", value); break;
		case GTSL::Gamepad::Side::LEFT: Get()->GetInputManager()->RecordLinearInputSource("Controller", controller, "LeftTrigger", value); break;
		default: break;
		}
	};

	auto vectors = [&](GTSL::Gamepad::Side side, const GTSL::Vector2 value)
	{
		switch (side)
		{
		case GTSL::Gamepad::Side::RIGHT: Get()->GetInputManager()->Record2DInputSource("Controller", controller, "RightStick", value); break;
		case GTSL::Gamepad::Side::LEFT: Get()->GetInputManager()->Record2DInputSource("Controller", controller, "LeftStick", value); break;
		default: break;
		}
	};
	
	GTSL::Update(gamepad, button, floats, vectors, 0);
	
	gamepad.SetVibration(0, 0);
}

void GameApplication::Shutdown()
{	
	Application::Shutdown();
}

void GameApplication::SetupInputSources()
{
	RegisterMouse();
	RegisterKeyboard();
	RegisterControllers();
}

void GameApplication::RegisterMouse()
{
	mouse = inputManagerInstance->RegisterInputDevice("Mouse");
	
	inputManagerInstance->Register2DInputSource(mouse, "MouseMove");

	inputManagerInstance->RegisterActionInputSource(mouse, "LeftMouseButton");
	inputManagerInstance->RegisterActionInputSource(mouse, "RightMouseButton");
	inputManagerInstance->RegisterActionInputSource(mouse, "MiddleMouseButton");

	inputManagerInstance->RegisterLinearInputSource(mouse, "MouseWheel");
}

void GameApplication::RegisterKeyboard()
{
	keyboard = inputManagerInstance->RegisterInputDevice("Keyboard");

	inputManagerInstance->RegisterCharacterInputSource(keyboard, "Character");
	
	inputManagerInstance->RegisterActionInputSource(keyboard, "Q_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "W_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "E_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "R_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "T_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "Y_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "U_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "I_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "O_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "P_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "A_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "S_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "D_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "F_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "G_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "H_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "J_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "K_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "L_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "Z_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "X_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "C_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "V_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "B_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "N_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "M_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "0_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "1_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "2_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "3_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "4_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "5_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "6_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "7_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "8_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "9_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "Backspace_Key");		inputManagerInstance->RegisterActionInputSource(keyboard, "Enter_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "Supr_Key");			inputManagerInstance->RegisterActionInputSource(keyboard, "Tab_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "CapsLock_Key");		inputManagerInstance->RegisterActionInputSource(keyboard, "Esc_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "RightShift_Key");	inputManagerInstance->RegisterActionInputSource(keyboard, "LeftShift_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "RightControl_Key");	inputManagerInstance->RegisterActionInputSource(keyboard, "LeftControl_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "RightAlt_Key");		inputManagerInstance->RegisterActionInputSource(keyboard, "LeftAlt_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "UpArrow_Key");		inputManagerInstance->RegisterActionInputSource(keyboard, "RightArrow_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "DownArrow_Key");		inputManagerInstance->RegisterActionInputSource(keyboard, "LeftArrow_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "SpaceBar_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad0_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad1_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad2_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad3_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad4_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad5_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad6_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad7_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad8_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "Numpad9_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "F1_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, "F2_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "F3_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, "F4_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "F5_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, "F6_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "F7_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, "F8_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "F9_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, "F10_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, "F11_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, "F12_Key");
}

void GameApplication::RegisterControllers()
{
	controller = inputManagerInstance->RegisterInputDevice("Controller");
	
	inputManagerInstance->Register2DInputSource(controller, "LeftStick");
	inputManagerInstance->Register2DInputSource(controller, "RightStick");

	inputManagerInstance->RegisterActionInputSource(controller, "TopFrontButton");
	inputManagerInstance->RegisterActionInputSource(controller, "RightFrontButton");
	inputManagerInstance->RegisterActionInputSource(controller, "BottomFrontButton");
	inputManagerInstance->RegisterActionInputSource(controller, "LeftFrontButton");

	inputManagerInstance->RegisterActionInputSource(controller, "TopDPadButton");
	inputManagerInstance->RegisterActionInputSource(controller, "RightDPadButton");
	inputManagerInstance->RegisterActionInputSource(controller, "BottomDPadButton");
	inputManagerInstance->RegisterActionInputSource(controller, "LeftDPadButton");
	
	inputManagerInstance->RegisterActionInputSource(controller, "LeftStickButton");
	inputManagerInstance->RegisterActionInputSource(controller, "RightStickButton");
	
	inputManagerInstance->RegisterActionInputSource(controller, "LeftMenuButton");
	inputManagerInstance->RegisterActionInputSource(controller, "RightMenuButton");
	
	inputManagerInstance->RegisterActionInputSource(controller, "LeftHatButton");
	inputManagerInstance->RegisterActionInputSource(controller, "RightHatButton");
	
	inputManagerInstance->RegisterLinearInputSource(controller, "LeftTrigger");
	inputManagerInstance->RegisterLinearInputSource(controller, "RightTrigger");
}

using namespace GTSL;

void GameApplication::onWindowResize(const GTSL::Extent2D& extent)
{
	Array<TaskDependency, 10> taskDependencies = { { "RenderSystem", AccessTypes::READ_WRITE }, { "RenderOrchestrator", AccessTypes::READ_WRITE },
	{ "MaterialSystem", AccessTypes::READ_WRITE } };

	auto ext = extent;

	auto resize = [](TaskInfo info, Extent2D newSize)
	{
		auto* renderSystem = info.GameInstance->GetSystem<RenderSystem>("RenderSystem");
		auto* renderOrchestrator = info.GameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");
		auto* materialSystem = info.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");

		renderSystem->OnResize(newSize);
		renderOrchestrator->OnResize(renderSystem, materialSystem, newSize);
	};
	
	if (extent != 0 && extent != oldSize)
	{
		gameInstance->AddDynamicTask("windowResize", Delegate<void(TaskInfo, Extent2D)>::Create(resize), taskDependencies, "FrameStart", "RenderStart", MoveRef(ext));
		oldSize = extent;
		BE_LOG_MESSAGE("Resize");
	}
}

void GameApplication::keyboardEvent(const GTSL::Window::KeyboardKeys key, const bool state, bool isFirstkeyOfType)
{
	Id id;
	
	switch (key)
	{
	case GTSL::Window::KeyboardKeys::Q: id = "Q_Key"; break;
	case GTSL::Window::KeyboardKeys::W: id = "W_Key"; break;
	case GTSL::Window::KeyboardKeys::E: id = "E_Key"; break;
	case GTSL::Window::KeyboardKeys::R: id = "R_Key"; break;
	case GTSL::Window::KeyboardKeys::T: id = "T_Key"; break;
	case GTSL::Window::KeyboardKeys::Y: id = "Y_Key"; break;
	case GTSL::Window::KeyboardKeys::U: id = "U_Key"; break;
	case GTSL::Window::KeyboardKeys::I: id = "I_Key"; break;
	case GTSL::Window::KeyboardKeys::O: id = "O_Key"; break;
	case GTSL::Window::KeyboardKeys::P: id = "P_Key"; break;
	case GTSL::Window::KeyboardKeys::A: id = "A_Key"; break;
	case GTSL::Window::KeyboardKeys::S: id = "S_Key"; break;
	case GTSL::Window::KeyboardKeys::D: id = "D_Key"; break;
	case GTSL::Window::KeyboardKeys::F: id = "F_Key"; break;
	case GTSL::Window::KeyboardKeys::G: id = "G_Key"; break;
	case GTSL::Window::KeyboardKeys::H: id = "H_Key"; break;
	case GTSL::Window::KeyboardKeys::J: id = "J_Key"; break;
	case GTSL::Window::KeyboardKeys::K: id = "K_Key"; break;
	case GTSL::Window::KeyboardKeys::L: id = "L_Key"; break;
	case GTSL::Window::KeyboardKeys::Z: id = "Z_Key"; break;
	case GTSL::Window::KeyboardKeys::X: id = "X_Key"; break;
	case GTSL::Window::KeyboardKeys::C: id = "C_Key"; break;
	case GTSL::Window::KeyboardKeys::V: id = "V_Key"; break;
	case GTSL::Window::KeyboardKeys::B: id = "B_Key"; break;
	case GTSL::Window::KeyboardKeys::N: id = "N_Key"; break;
	case GTSL::Window::KeyboardKeys::M: id = "M_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard0: id = "0_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard1: id = "1_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard2: id = "2_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard3: id = "3_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard4: id = "4_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard5: id = "5_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard6: id = "6_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard7: id = "7_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard8: id = "8_Key"; break;
	case GTSL::Window::KeyboardKeys::Keyboard9: id = "9_Key"; break;
	case GTSL::Window::KeyboardKeys::Backspace: id = "Backspace_Key"; break;
	case GTSL::Window::KeyboardKeys::Enter: id = "Enter_Key"; break;
	case GTSL::Window::KeyboardKeys::Supr: id = "Supr_Key"; break;
	case GTSL::Window::KeyboardKeys::Tab: id = "Tab_Key"; break;
	case GTSL::Window::KeyboardKeys::CapsLock: id = "CapsLock_Key"; break;
	case GTSL::Window::KeyboardKeys::Esc: id = "Esc_Key"; break;
	case GTSL::Window::KeyboardKeys::RShift: id = "RightShift_Key"; break;
	case GTSL::Window::KeyboardKeys::LShift: id = "LeftShift_Key"; break;
	case GTSL::Window::KeyboardKeys::RControl: id = "RightControl_Key"; break;
	case GTSL::Window::KeyboardKeys::LControl: id = "LeftControl_Key"; break;
	case GTSL::Window::KeyboardKeys::Alt: id = "LeftAlt_Key"; break;
	case GTSL::Window::KeyboardKeys::AltGr: id = "RightAlt_Key"; break;
	case GTSL::Window::KeyboardKeys::UpArrow: id = "Up_Key"; break;
	case GTSL::Window::KeyboardKeys::RightArrow: id = "Right_Key"; break;
	case GTSL::Window::KeyboardKeys::DownArrow: id = "Down_Key"; break;
	case GTSL::Window::KeyboardKeys::LeftArrow: id = "Left_Key"; break;
	case GTSL::Window::KeyboardKeys::SpaceBar: id = "SpaceBar_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad0: id = "Numpad0_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad1: id = "Numpad1_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad2: id = "Numpad2_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad3: id = "Numpad3_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad4: id = "Numpad4_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad5: id = "Numpad5_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad6: id = "Numpad6_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad7: id = "Numpad7_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad8: id = "Numpad8_Key"; break;
	case GTSL::Window::KeyboardKeys::Numpad9: id = "Numpad9_Key"; break;
	case GTSL::Window::KeyboardKeys::F1: id = "F1_Key"; break;
	case GTSL::Window::KeyboardKeys::F2: id = "F2_Key"; break;
	case GTSL::Window::KeyboardKeys::F3: id = "F3_Key"; break;
	case GTSL::Window::KeyboardKeys::F4: id = "F4_Key"; break;
	case GTSL::Window::KeyboardKeys::F5: id = "F5_Key"; break;
	case GTSL::Window::KeyboardKeys::F6: id = "F6_Key"; break;
	case GTSL::Window::KeyboardKeys::F7: id = "F7_Key"; break;
	case GTSL::Window::KeyboardKeys::F8: id = "F8_Key"; break;
	case GTSL::Window::KeyboardKeys::F9: id = "F9_Key"; break;
	case GTSL::Window::KeyboardKeys::F10: id = "F10_Key"; break;
	case GTSL::Window::KeyboardKeys::F11: id = "F11_Key"; break;
	case GTSL::Window::KeyboardKeys::F12: id = "F12_Key"; break;
	default: break;
	}

	if (isFirstkeyOfType)
	{
		GetInputManager()->RecordActionInputSource("Keyboard", keyboard, id, state);
	}
}

void GameApplication::windowUpdateFunction(void* userData, GTSL::Window::WindowEvents event, void* eventData)
{
	auto* app = static_cast<GameApplication*>(userData);

	switch (event)
	{
	case Window::WindowEvents::FOCUS:
	{
		auto* focusEventData = static_cast<GTSL::Window::FocusEventData*>(eventData);
		if(focusEventData->Focus) {
			app->gameInstance->DispatchEvent("Application", EventHandle<bool>("OnFocusGain"), GTSL::MoveRef(focusEventData->HadFocus));
		}
		else {
			app->gameInstance->DispatchEvent("Application", EventHandle<bool>("OnFocusLoss"), GTSL::MoveRef(focusEventData->HadFocus));
		}
		break;
	}
	case GTSL::Window::WindowEvents::CLOSE: app->Close(CloseMode::OK, {}); break;
	case GTSL::Window::WindowEvents::KEYBOARD_KEY:
	{
		auto* keyboardEventData = static_cast<GTSL::Window::KeyboardKeyEventData*>(eventData);
		app->keyboardEvent(keyboardEventData->Key, keyboardEventData->State, keyboardEventData->IsFirstTime);
		break;
	}
	case GTSL::Window::WindowEvents::CHAR: app->GetInputManager()->RecordCharacterInputSource("Keyboard", app->keyboard, "Character", *(GTSL::Window::CharEventData*)eventData); break;
	case GTSL::Window::WindowEvents::SIZE:
	{
		auto* sizingEventData = static_cast<GTSL::Window::WindowSizeEventData*>(eventData);
		app->onWindowResize(*sizingEventData);
		break;
	}
	case GTSL::Window::WindowEvents::MOVING: break;
	case GTSL::Window::WindowEvents::MOUSE_MOVE:
	{
		auto* mouseMoveEventData = static_cast<GTSL::Window::MouseMoveEventData*>(eventData);
		app->GetInputManager()->Record2DInputSource("Mouse", app->mouse, "MouseMove", *mouseMoveEventData);
		break;
	}
	case GTSL::Window::WindowEvents::MOUSE_WHEEL:
	{
		auto* mouseWheelEventData = static_cast<GTSL::Window::MouseWheelEventData*>(eventData);
		app->GetInputManager()->RecordLinearInputSource("Mouse", app->mouse, "MouseWheel", *mouseWheelEventData);
		break;
	}
	case GTSL::Window::WindowEvents::MOUSE_BUTTON:
	{
		auto* mouseButtonEventData = static_cast<GTSL::Window::MouseButtonEventData*>(eventData);

		switch (mouseButtonEventData->Button)
		{
		case GTSL::Window::MouseButton::LEFT_BUTTON:
			app->GetInputManager()->RecordActionInputSource("Mouse", app->mouse, "LeftMouseButton", mouseButtonEventData->State);
			app->GetGameInstance()->GetSystem<CanvasSystem>("CanvasSystem")->SignalHit(GTSL::Vector2());
			break;
		case GTSL::Window::MouseButton::RIGHT_BUTTON: app->GetInputManager()->RecordActionInputSource("Mouse", app->mouse, "RightMouseButton", mouseButtonEventData->State); break;
		case GTSL::Window::MouseButton::MIDDLE_BUTTON: app->GetInputManager()->RecordActionInputSource("Mouse", app->mouse, "MiddleMouseButton", mouseButtonEventData->State); break;
		default:;
		}
		break;
	}
	default:;
	}
}
