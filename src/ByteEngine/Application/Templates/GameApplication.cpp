#include "GameApplication.h"

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Application/WindowSystem.hpp"
#include "ByteEngine/Debug/FunctionTimer.h"
#include "ByteEngine/Game/CameraSystem.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Physics/PhysicsWorld.h"
#include "ByteEngine/Game/WorldSystem.hpp"
#include "ByteEngine/Render/LightsRenderGroup.h"
#include "ByteEngine/Render/RenderOrchestrator.h"
#include "ByteEngine/Render/StaticMeshSystem.h"
#include "ByteEngine/Network/ConnectionHandler.hpp"

#include "ByteEngine/Render/RenderSystem.h"
#include "ByteEngine/Render/WorldRenderPipeline.hpp"
#include "ByteEngine/Render/UIManager.h"

#include "ByteEngine/Resources/ShaderResourceManager.h"
#include "ByteEngine/Resources/PipelineCacheResourceManager.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"
#include "ByteEngine/Resources/AudioResourceManager.h"
#include "ByteEngine/Resources/FontResourceManager.h"
#include "ByteEngine/Resources/CurvesResourceManager.hpp"

#include "ByteEngine/Sound/AudioSystem.h"
#include "GTSL/System.h"

class RenderOrchestrator;

bool GameApplication::Initialize()
{
	if(!Application::Initialize()) { return false; } 
	
	SetupInputSources();
	
	applicationManager->AddSystem<StaticMeshResourceManager>(u8"StaticMeshResourceManager");
	applicationManager->AddSystem<TextureResourceManager>(u8"TextureResourceManager");
	applicationManager->AddSystem<ShaderResourceManager>(u8"ShaderResourceManager");
	applicationManager->AddSystem<AudioResourceManager>(u8"AudioResourceManager");
	applicationManager->AddSystem<PipelineCacheResourceManager>(u8"PipelineCacheResourceManager");
	applicationManager->AddSystem<FontResourceManager>(u8"FontResourceManager");

	return true;
}

void GameApplication::PostInitialize()
{
	//FRAME START
	applicationManager->AddStage(u8"FrameStart");

	//GAMEPLAY CODE BEGINS
	applicationManager->AddStage(u8"GameplayStart");
	//GAMEPLAY CODE ENDS
	applicationManager->AddStage(u8"GameplayEnd");

	applicationManager->AddStage(u8"PostGameplay");
	applicationManager->AddStage(u8"PreRenderSetup");
	
	//RENDER CODE BEGINS
	applicationManager->AddStage(u8"RenderSetup");
	//RENDER IS DISPATCHED
	applicationManager->AddStage(u8"Render");
	
	//FRAME ENDS
	applicationManager->AddStage(u8"FrameEnd");

	applicationManager->AddEvent(u8"Application", EventHandle(u8"OnFocusGain"));
	applicationManager->AddEvent(u8"Application", EventHandle(u8"OnFocusLoss"));
	applicationManager->AddEvent(u8"Application", EventHandle<GTSL::Extent2D>(u8"OnWindowResize"));
	
	auto* windowSystem = applicationManager->AddSystem<WindowSystem>(u8"WindowSystem");

	{
		bool fullscreen = GetBoolOption(u8"fullScreen");
		GTSL::Extent2D screenSize;

		if(fullscreen) {
			screenSize = GTSL::System::GetScreenExtent();			
		} else {
			screenSize = GetExtent2DOption(u8"resolution");
		}

		windowHandle = windowSystem->CreateWindow(u8"main", u8"Sandbox", screenSize);
	}

	auto* renderSystem = applicationManager->AddSystem<RenderSystem>(u8"RenderSystem");
	auto* renderOrchestrator = applicationManager->AddSystem<RenderOrchestrator>(u8"RenderOrchestrator");

	applicationManager->AddSystem<StaticMeshSystem>(u8"StaticMeshSystem");
	applicationManager->AddSystem<WorldRendererPipeline>(u8"WorldRendererPipeline");

	applicationManager->AddSystem<PhysicsWorld>(u8"PhysicsWorld");
	applicationManager->AddSystem<AudioSystem>(u8"AudioSystem");

	applicationManager->AddSystem<CameraSystem>(u8"CameraSystem");

	applicationManager->AddSystem<LightsRenderGroup>(u8"LightsRenderGroup");

	windowSystemHandle = applicationManager->GetSystemReference(u8"WindowSystem");

	windowSystem->keyboard = keyboard; windowSystem->mouse = mouse;
	
	auto* uiManager = applicationManager->AddSystem<UIManager>(u8"UIManager");

	applicationManager->AddSystem<UIRenderManager>(u8"UIRenderManager");

	applicationManager->AddSystem<WorldSystem>(u8"WorldSystem");
}	

void GameApplication::OnUpdate(const OnUpdateInfo& updateInfo)
{
	Application::OnUpdate(updateInfo);

	auto* windowSystem = applicationManager->GetSystem<WindowSystem>(windowSystemHandle);

	windowSystem->Update();

	mouseCount = 0;

	auto gamePadUpdate = [&](GTSL::Gamepad::SourceNames source, GTSL::Gamepad::Side side, const void* value) {
		switch (source) {
		case GTSL::Gamepad::SourceNames::TRIGGER: {
			const auto state = *static_cast<const float32*>(value);

			switch (side) {
			case GTSL::Gamepad::Side::RIGHT: {
				GetInputManager()->RecordInputSource(controller, u8"RightTrigger", state);

				break;
			}
			case GTSL::Gamepad::Side::LEFT: {
				GetInputManager()->RecordInputSource(controller, u8"LeftTrigger", state);

				break;
			}
			default: break;
			}

			break;
		}
		case GTSL::Gamepad::SourceNames::SHOULDER: {
			auto state = *static_cast<const bool*>(value);
			switch (side) {
			case GTSL::Gamepad::Side::RIGHT: GetInputManager()->RecordInputSource(controller, u8"RightHatButton", state); break;
			case GTSL::Gamepad::Side::LEFT: GetInputManager()->RecordInputSource(controller, u8"LeftHatButton", state); break;
			default: __debugbreak();
			}
			break;
		}
		case GTSL::Gamepad::SourceNames::THUMB: {
			auto state = *static_cast<const GTSL::Vector2*>(value);
			switch (side) {
			case GTSL::Gamepad::Side::RIGHT: Get()->GetInputManager()->RecordInputSource(controller, u8"RightStick", state); break;
			case GTSL::Gamepad::Side::LEFT: Get()->GetInputManager()->RecordInputSource(controller, u8"LeftStick", state); break;
			default: __debugbreak();
			}

			break;
		}
		case GTSL::Gamepad::SourceNames::MIDDLE_BUTTONS: {
			auto state = *static_cast<const bool*>(value);
			switch (side) {
			case GTSL::Gamepad::Side::RIGHT: GetInputManager()->RecordInputSource(controller, u8"RightMenuButton", state); break;
			case GTSL::Gamepad::Side::LEFT: GetInputManager()->RecordInputSource(controller, u8"LeftMenuButton", state); break;
			default: __debugbreak();
			}
			break;
		}
		case GTSL::Gamepad::SourceNames::DPAD: {
			auto state = *static_cast<const bool*>(value);
			switch (side) {
			case GTSL::Gamepad::Side::UP: GetInputManager()->RecordInputSource(controller, u8"TopDPadButton", state); break;
			case GTSL::Gamepad::Side::RIGHT: GetInputManager()->RecordInputSource(controller, u8"RightDPadButton", state); break;
			case GTSL::Gamepad::Side::DOWN: GetInputManager()->RecordInputSource(controller, u8"BottomDPadButton", state); break;
			case GTSL::Gamepad::Side::LEFT: GetInputManager()->RecordInputSource(controller, u8"LeftDPadButton", state); break;
			default: ;
			}
			break;
		}
		case GTSL::Gamepad::SourceNames::FACE_BUTTONS: {
			auto state = *static_cast<const bool*>(value);
			switch (side) {
			case GTSL::Gamepad::Side::UP: GetInputManager()->RecordInputSource(controller, u8"TopFrontButton", state); break;
			case GTSL::Gamepad::Side::RIGHT: GetInputManager()->RecordInputSource(controller, u8"RightFrontButton", state); break;
			case GTSL::Gamepad::Side::DOWN: GetInputManager()->RecordInputSource(controller, u8"BottomFrontButton", state); break;
			case GTSL::Gamepad::Side::LEFT: GetInputManager()->RecordInputSource(controller, u8"LeftFrontButton", state); break;
			default: __debugbreak();
			}

			break;
		}
		case GTSL::Gamepad::SourceNames::THUMB_BUTTONS: {
			auto state = *static_cast<const bool*>(value);
			switch (side) {
			case GTSL::Gamepad::Side::RIGHT: GetInputManager()->RecordInputSource(controller, u8"RightStickButton", state); break;
			case GTSL::Gamepad::Side::LEFT: GetInputManager()->RecordInputSource(controller, u8"LeftStickButton", state); break;
			default: __debugbreak();
			}
			break;
		}
		default: __debugbreak(); break;
		}
	};
	
	GTSL::Gamepad::Update(gamepad, gamePadUpdate, 0);

	{
		auto lowEndVibration = inputManagerInstance->GetInputDeviceParameter(controller, u8"LowEndVibration");
		auto highEndVibration = inputManagerInstance->GetInputDeviceParameter(controller, u8"HighEndVibration");
		//gamepad.SetVibration(lowEndVibration, highEndVibration);
	}
}

void GameApplication::Shutdown()
{	
	Application::Shutdown();
}

void GameApplication::SetupInputSources() {
	RegisterMouse();
	RegisterKeyboard();
	RegisterControllers();
}

void GameApplication::RegisterMouse() {
	mouse = inputManagerInstance->RegisterInputDevice(u8"Mouse");
	
	inputManagerInstance->RegisterInputSource(mouse, u8"MouseMove", InputManager::Type::VECTOR2D);
	inputManagerInstance->RegisterInputSource(mouse, u8"LeftMouseButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(mouse, u8"RightMouseButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(mouse, u8"MiddleMouseButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(mouse, u8"MouseWheel", InputManager::Type::LINEAR);
}

void GameApplication::RegisterKeyboard()
{
	keyboard = inputManagerInstance->RegisterInputDevice(u8"Keyboard");

	auto keys = GTSL::StaticVector<GTSL::ShortString<64>, 128>{ u8"Q_Key", u8"W_Key", u8"E_Key", u8"R_Key", u8"T_Key", u8"Y_Key", u8"U_Key", u8"I_Key", u8"O_Key", u8"P_Key",
	u8"A_Key", u8"S_Key", u8"D_Key", u8"F_Key", u8"G_Key", u8"H_Key", u8"J_Key", u8"K_Key", u8"L_Key",
	u8"Z_Key", u8"X_Key", u8"C_Key", u8"V_Key", u8"B_Key", u8"N_Key", u8"M_Key",
	u8"0_Key", u8"1_Key", u8"2_Key", u8"3_Key", u8"4_Key", u8"5_Key", u8"6_Key", u8"7_Key", u8"8_Key", u8"9_Key",
	u8"Backspace_Key", u8"Enter_Key", u8"Supr_Key", u8"Tab_Key", u8"CapsLock_Key", u8"Esc_Key", u8"SpaceBar_Key",
	u8"LeftShift_Key", u8"RightShift_Key", u8"LeftControl_Key", u8"RightControl_Key", u8"LeftAlt_Key", u8"RightAlt_Key",
	u8"UpArrow_Key", u8"RightArrow_Key", u8"DownArrow_Key", u8"LeftArrow_Key",
	u8"Numpad0_Key", u8"Numpad1_Key", u8"Numpad2_Key", u8"Numpad3_Key", u8"Numpad4_Key", u8"Numpad5_Key", u8"Numpad6_Key", u8"Numpad7_Key", u8"Numpad8_Key", u8"Numpad9_Key",
	u8"F1_Key", u8"F2_Key", u8"F3_Key", u8"F4_Key", u8"F5_Key", u8"F6_Key", u8"F7_Key", u8"F8_Key", u8"F9_Key", u8"F10_Key", u8"F11_Key", u8"F12_Key" };

	GTSL::StaticVector<GTSL::StringView, 128> views;

	for(auto& e : keys) {
		views.EmplaceBack(e);
	}

	inputManagerInstance->RegisterInputSources(keyboard, views, InputManager::Type::BOOL);
}

void GameApplication::RegisterControllers()
{
	controller = inputManagerInstance->RegisterInputDevice(u8"Controller");

	inputManagerInstance->RegisterInputDeviceParameter(controller, u8"LowEndVibration");
	inputManagerInstance->RegisterInputDeviceParameter(controller, u8"HighEndVibration");
	
	inputManagerInstance->RegisterInputSource(controller, u8"LeftStick", InputManager::Type::VECTOR2D);
	inputManagerInstance->RegisterInputSource(controller, u8"RightStick", InputManager::Type::VECTOR2D);

	inputManagerInstance->RegisterInputSource(controller, u8"LeftTrigger", InputManager::Type::LINEAR);
	inputManagerInstance->RegisterInputSource(controller, u8"RightTrigger", InputManager::Type::LINEAR);

	inputManagerInstance->RegisterInputSource(controller, u8"TopFrontButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"RightFrontButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"BottomFrontButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"LeftFrontButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"TopDPadButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"RightDPadButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"BottomDPadButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"LeftDPadButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"LeftStickButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"RightStickButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"LeftMenuButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"RightMenuButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"LeftHatButton", InputManager::Type::BOOL);
	inputManagerInstance->RegisterInputSource(controller, u8"RightHatButton", InputManager::Type::BOOL);
}

using namespace GTSL;

void GameApplication::keyboardEvent(const Window::KeyboardKeys key, const bool state, bool isFirstkeyOfType) {
	GTSL::StaticString<64> id;
	
	switch (key) {
	case Window::KeyboardKeys::Q: id = u8"Q_Key"; break; case Window::KeyboardKeys::W: id = u8"W_Key"; break;
	case Window::KeyboardKeys::E: id = u8"E_Key"; break; case Window::KeyboardKeys::R: id = u8"R_Key"; break;
	case Window::KeyboardKeys::T: id = u8"T_Key"; break; case Window::KeyboardKeys::Y: id = u8"Y_Key"; break;
	case Window::KeyboardKeys::U: id = u8"U_Key"; break; case Window::KeyboardKeys::I: id = u8"I_Key"; break;
	case Window::KeyboardKeys::O: id = u8"O_Key"; break; case Window::KeyboardKeys::P: id = u8"P_Key"; break;
	case Window::KeyboardKeys::A: id = u8"A_Key"; break; case Window::KeyboardKeys::S: id = u8"S_Key"; break;
	case Window::KeyboardKeys::D: id = u8"D_Key"; break; case Window::KeyboardKeys::F: id = u8"F_Key"; break;
	case Window::KeyboardKeys::G: id = u8"G_Key"; break; case Window::KeyboardKeys::H: id = u8"H_Key"; break;
	case Window::KeyboardKeys::J: id = u8"J_Key"; break; case Window::KeyboardKeys::K: id = u8"K_Key"; break;
	case Window::KeyboardKeys::L: id = u8"L_Key"; break; case Window::KeyboardKeys::Z: id = u8"Z_Key"; break;
	case Window::KeyboardKeys::X: id = u8"X_Key"; break; case Window::KeyboardKeys::C: id = u8"C_Key"; break;
	case Window::KeyboardKeys::V: id = u8"V_Key"; break; case Window::KeyboardKeys::B: id = u8"B_Key"; break;
	case Window::KeyboardKeys::N: id = u8"N_Key"; break; case Window::KeyboardKeys::M: id = u8"M_Key"; break;
	case Window::KeyboardKeys::Keyboard0: id = u8"0_Key"; break; case Window::KeyboardKeys::Keyboard1: id = u8"1_Key"; break;
	case Window::KeyboardKeys::Keyboard2: id = u8"2_Key"; break; case Window::KeyboardKeys::Keyboard3: id = u8"3_Key"; break;
	case Window::KeyboardKeys::Keyboard4: id = u8"4_Key"; break; case Window::KeyboardKeys::Keyboard5: id = u8"5_Key"; break;
	case Window::KeyboardKeys::Keyboard6: id = u8"6_Key"; break; case Window::KeyboardKeys::Keyboard7: id = u8"7_Key"; break;
	case Window::KeyboardKeys::Keyboard8: id = u8"8_Key"; break; case Window::KeyboardKeys::Keyboard9: id = u8"9_Key"; break;
	case Window::KeyboardKeys::Backspace: id = u8"Backspace_Key"; break;
	case Window::KeyboardKeys::Enter: id = u8"Enter_Key"; break;
	case Window::KeyboardKeys::Supr: id = u8"Supr_Key"; break;
	case Window::KeyboardKeys::Tab: id = u8"Tab_Key"; break;
	case Window::KeyboardKeys::CapsLock: id = u8"CapsLock_Key"; break;
	case Window::KeyboardKeys::Esc: id = u8"Esc_Key"; break;
	case Window::KeyboardKeys::RShift: id = u8"RightShift_Key"; break; case Window::KeyboardKeys::LShift: id = u8"LeftShift_Key"; break;
	case Window::KeyboardKeys::RControl: id = u8"RightControl_Key"; break; case Window::KeyboardKeys::LControl: id = u8"LeftControl_Key"; break;
	case Window::KeyboardKeys::Alt: id = u8"LeftAlt_Key"; break; case Window::KeyboardKeys::AltGr: id = u8"RightAlt_Key"; break;
	case Window::KeyboardKeys::UpArrow: id = u8"Up_Key"; break; case Window::KeyboardKeys::RightArrow: id = u8"Right_Key"; break;
	case Window::KeyboardKeys::DownArrow: id = u8"Down_Key"; break; case Window::KeyboardKeys::LeftArrow: id = u8"Left_Key"; break;
	case Window::KeyboardKeys::SpaceBar: id = u8"SpaceBar_Key"; break;
	case Window::KeyboardKeys::Numpad0: id = u8"Numpad0_Key"; break; case Window::KeyboardKeys::Numpad1: id = u8"Numpad1_Key"; break;
	case Window::KeyboardKeys::Numpad2: id = u8"Numpad2_Key"; break; case Window::KeyboardKeys::Numpad3: id = u8"Numpad3_Key"; break;
	case Window::KeyboardKeys::Numpad4: id = u8"Numpad4_Key"; break; case Window::KeyboardKeys::Numpad5: id = u8"Numpad5_Key"; break;
	case Window::KeyboardKeys::Numpad6: id = u8"Numpad6_Key"; break; case Window::KeyboardKeys::Numpad7: id = u8"Numpad7_Key"; break;
	case Window::KeyboardKeys::Numpad8: id = u8"Numpad8_Key"; break; case Window::KeyboardKeys::Numpad9: id = u8"Numpad9_Key"; break;
	case Window::KeyboardKeys::F1: id = u8"F1_Key"; break; case Window::KeyboardKeys::F2: id = u8"F2_Key"; break;
	case Window::KeyboardKeys::F3: id = u8"F3_Key"; break; case Window::KeyboardKeys::F4: id = u8"F4_Key"; break;
	case Window::KeyboardKeys::F5: id = u8"F5_Key"; break; case Window::KeyboardKeys::F6: id = u8"F6_Key"; break;
	case Window::KeyboardKeys::F7: id = u8"F7_Key"; break; case Window::KeyboardKeys::F8: id = u8"F8_Key"; break;
	case Window::KeyboardKeys::F9: id = u8"F9_Key"; break; case Window::KeyboardKeys::F10: id = u8"F10_Key"; break;
	case Window::KeyboardKeys::F11: id = u8"F11_Key"; break; case Window::KeyboardKeys::F12: id = u8"F12_Key"; break;
	default: break;
	}

	if (isFirstkeyOfType) {
		GetInputManager()->RecordInputSource(keyboard, id, state);
	}
}
