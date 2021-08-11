#include "GameApplication.h"

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Debug/FunctionTimer.h"
#include "ByteEngine/Game/CameraSystem.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Render/LightsRenderGroup.h"
#include "ByteEngine/Render/RenderOrchestrator.h"
#include "ByteEngine/Render/StaticMeshRenderGroup.h"

#include "ByteEngine/Render/RenderSystem.h"
#include "ByteEngine/Render/UIManager.h"

#include "ByteEngine/Resources/ShaderResourceManager.h"
#include "ByteEngine/Resources/PipelineCacheResourceManager.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"
#include "ByteEngine/Resources/AudioResourceManager.h"
#include "ByteEngine/Resources/FontResourceManager.h"

#include "ByteEngine/Sound/AudioSystem.h"

class RenderOrchestrator;

bool GameApplication::Initialize()
{
	if(!Application::Initialize()) { return false; } 
	
	SetupInputSources();
	
	CreateResourceManager<StaticMeshResourceManager>();
	CreateResourceManager<TextureResourceManager>();
	CreateResourceManager<ShaderResourceManager>();
	CreateResourceManager<AudioResourceManager>();
	CreateResourceManager<PipelineCacheResourceManager>();
	//CreateResourceManager<AnimationResourceManager>();

	return true;
}

void GameApplication::PostInitialize()
{
	//FRAME START
	gameInstance->AddStage(u8"FrameStart");

	//GAMEPLAY CODE BEGINS
	gameInstance->AddStage(u8"GameplayStart");
	//GAMEPLAY CODE ENDS
	gameInstance->AddStage(u8"GameplayEnd");
	
	//RENDER CODE BEGINS
	gameInstance->AddStage(u8"RenderStart");
	//RENDER SETUP BEGINS
	gameInstance->AddStage(u8"RenderStartSetup");
	//RENDER SETUP ENDS
	gameInstance->AddStage(u8"RenderEndSetup");
	//RENDER IS DISPATCHED
	gameInstance->AddStage(u8"RenderDo");
	//RENDER DISPATCH IS DONE
	gameInstance->AddStage(u8"RenderFinished");
	//RENDER CODE ENDS
	gameInstance->AddStage(u8"RenderEnd");
	
	//FRAME ENDS
	gameInstance->AddStage(u8"FrameEnd");

	gameInstance->AddEvent(u8"Application", EventHandle(u8"OnFocusGain"));
	gameInstance->AddEvent(u8"Application", EventHandle(u8"OnFocusLoss"));
	
	auto* renderSystem = gameInstance->AddSystem<RenderSystem>(u8"RenderSystem");
	auto* renderOrchestrator = gameInstance->AddSystem<RenderOrchestrator>(u8"RenderOrchestrator");

	gameInstance->AddSystem<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup");
	gameInstance->AddSystem<AudioSystem>(u8"AudioSystem");

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
	window.AddDevice(GTSL::Window::DeviceType::GAMEPAD);
	
	renderSystem->SetWindow(&window);

	window.SetWindowVisibility(true);
	
	gameInstance->AddSystem<CameraSystem>(u8"CameraSystem");
	
	{
		renderOrchestrator->AddAttachment(u8"Color", 8, 4, GAL::ComponentType::INT, GAL::TextureType::COLOR, GTSL::RGBA(0, 0, 0, 0));
		renderOrchestrator->AddAttachment(u8"Position", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR, GTSL::RGBA(0, 0, 0, 0));
		renderOrchestrator->AddAttachment(u8"Normal", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR, GTSL::RGBA(0, 0, 0, 0));
		renderOrchestrator->AddAttachment(u8"RenderDepth", 32, 1, GAL::ComponentType::FLOAT, GAL::TextureType::DEPTH, GTSL::RGBA(1.0f, 0, 0, 0));

		RenderOrchestrator::PassData geoRenderPass;
		geoRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		geoRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Color" } ); //result attachment
		geoRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Position" } );
		geoRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Normal" } );
		geoRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"RenderDepth" } );
		renderOrchestrator->AddPass(u8"SceneRenderPass", renderOrchestrator->GetCameraDataLayer(), renderSystem, geoRenderPass);

		RenderOrchestrator::PassData uiRenderPass{};
		uiRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		uiRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Color" }); //result attachment

		RenderOrchestrator::PassData rtRenderPass{};
		rtRenderPass.PassType = RenderOrchestrator::PassType::RAY_TRACING;
		rtRenderPass.ReadAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Position" });
		rtRenderPass.ReadAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Normal" });
		rtRenderPass.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Color" }); //result attachment
		
		//renderOrchestrator->ToggleRenderPass("SceneRenderPass", true);
		//renderOrchestrator->ToggleRenderPass("UIRenderPass", false);
		//renderOrchestrator->ToggleRenderPass("SceneRTRenderPass", true);
	}

	
	auto* uiManager = gameInstance->AddSystem<UIManager>(u8"UIManager");
	gameInstance->AddSystem<CanvasSystem>(u8"CanvasSystem");
	
	gameInstance->AddSystem<StaticMeshRenderManager>(u8"StaticMeshRenderManager");
	gameInstance->AddSystem<UIRenderManager>(u8"UIRenderManager");
	gameInstance->AddSystem<LightsRenderGroup>(u8"LightsRenderGroup");
	
	renderOrchestrator->AddRenderManager(gameInstance, u8"StaticMeshRenderManager", gameInstance->GetSystemReference(u8"StaticMeshRenderManager"));
	renderOrchestrator->AddRenderManager(gameInstance, u8"UIRenderManager", gameInstance->GetSystemReference(u8"UIRenderManager"));
}	

void GameApplication::OnUpdate(const OnUpdateInfo& updateInfo)
{
	Application::OnUpdate(updateInfo);

	window.Update(this, GTSL::Delegate<void(void*, GTSL::Window::WindowEvents, void*)>::Create<GameApplication, &GameApplication::windowUpdateFunction>(this));

	auto gamePadUpdate = [&](GTSL::Gamepad::SourceNames source, GTSL::Gamepad::Side side, const void* value) {
		switch (source) {
		case GTSL::Gamepad::SourceNames::TRIGGER: {
			const auto state = *static_cast<const float32*>(value);

			constexpr float32 TRIGGER_THRESHOLD = 0.95f;

			switch (side) {
			case GTSL::Gamepad::Side::RIGHT: {
				GetInputManager()->RecordInputSource(controller, u8"RightTrigger", state);

				const auto wasPressed = GetInputManager()->GetActionInputSourceValue(controller, u8"LeftTrigger");

				if (state >= TRIGGER_THRESHOLD) {
					if (!wasPressed) {
						GetInputManager()->RecordInputSource(controller, u8"RightTrigger", true);
					}
				} else {
					if (wasPressed) {
						GetInputManager()->RecordInputSource(controller, u8"RightTrigger", false);
					}
				}

				break;
			}
			case GTSL::Gamepad::Side::LEFT: {
				GetInputManager()->RecordInputSource(controller, u8"LeftTrigger", state);

				const auto wasPressed = GetInputManager()->GetActionInputSourceValue(controller, u8"LeftTrigger");

				if (state >= TRIGGER_THRESHOLD) { //if is pressed
					if (!wasPressed) { //and wasn't pressed
						GetInputManager()->RecordInputSource(controller, u8"LeftTrigger", true);
					}
				} else { //isn't pressed
					if (wasPressed && state <= TRIGGER_THRESHOLD - 0.10f) {
						GetInputManager()->RecordInputSource(controller, u8"LeftTrigger", false);
					}
				}

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

void GameApplication::SetupInputSources()
{
	RegisterMouse();
	RegisterKeyboard();
	RegisterControllers();
}

void GameApplication::RegisterMouse()
{
	mouse = inputManagerInstance->RegisterInputDevice(u8"Mouse");
	
	inputManagerInstance->Register2DInputSource(mouse, u8"MouseMove");

	inputManagerInstance->RegisterActionInputSource(mouse, u8"LeftMouseButton");
	inputManagerInstance->RegisterActionInputSource(mouse, u8"RightMouseButton");
	inputManagerInstance->RegisterActionInputSource(mouse, u8"MiddleMouseButton");

	inputManagerInstance->RegisterLinearInputSource(mouse, u8"MouseWheel");
}

void GameApplication::RegisterKeyboard()
{
	keyboard = inputManagerInstance->RegisterInputDevice(u8"Keyboard");

	inputManagerInstance->RegisterCharacterInputSource(keyboard, u8"Character");
	
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Q_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"W_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"E_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"R_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"T_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"Y_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"U_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"I_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"O_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"P_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"A_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"S_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"D_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"F_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"G_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"H_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"J_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"K_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"L_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"Z_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"X_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"C_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"V_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"B_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"N_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"M_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"0_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"1_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"2_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"3_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"4_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"5_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"6_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"7_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"8_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"9_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Backspace_Key");	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Enter_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Supr_Key");			inputManagerInstance->RegisterActionInputSource(keyboard, u8"Tab_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"CapsLock_Key");		inputManagerInstance->RegisterActionInputSource(keyboard, u8"Esc_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"RightShift_Key");	inputManagerInstance->RegisterActionInputSource(keyboard, u8"LeftShift_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"RightControl_Key");	inputManagerInstance->RegisterActionInputSource(keyboard, u8"LeftControl_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"RightAlt_Key");		inputManagerInstance->RegisterActionInputSource(keyboard, u8"LeftAlt_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"UpArrow_Key");		inputManagerInstance->RegisterActionInputSource(keyboard, u8"RightArrow_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"DownArrow_Key");	inputManagerInstance->RegisterActionInputSource(keyboard, u8"LeftArrow_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"SpaceBar_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad0_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad1_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad2_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad3_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad4_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad5_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad6_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad7_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad8_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"Numpad9_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"F1_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, u8"F2_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"F3_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, u8"F4_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"F5_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, u8"F6_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"F7_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, u8"F8_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"F9_Key");  inputManagerInstance->RegisterActionInputSource(keyboard, u8"F10_Key");
	inputManagerInstance->RegisterActionInputSource(keyboard, u8"F11_Key"); inputManagerInstance->RegisterActionInputSource(keyboard, u8"F12_Key");
}

void GameApplication::RegisterControllers()
{
	controller = inputManagerInstance->RegisterInputDevice(u8"Controller");

	inputManagerInstance->RegisterInputDeviceParameter(controller, u8"LowEndVibration");
	inputManagerInstance->RegisterInputDeviceParameter(controller, u8"HighEndVibration");
	
	inputManagerInstance->Register2DInputSource(controller, u8"LeftStick");
	inputManagerInstance->Register2DInputSource(controller, u8"RightStick");

	inputManagerInstance->RegisterActionInputSource(controller, u8"TopFrontButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"RightFrontButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"BottomFrontButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"LeftFrontButton");
	
	inputManagerInstance->RegisterActionInputSource(controller, u8"LeftTrigger");
	inputManagerInstance->RegisterActionInputSource(controller, u8"RightTrigger");

	inputManagerInstance->RegisterActionInputSource(controller, u8"TopDPadButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"RightDPadButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"BottomDPadButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"LeftDPadButton");
	
	inputManagerInstance->RegisterActionInputSource(controller, u8"LeftStickButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"RightStickButton");
	
	inputManagerInstance->RegisterActionInputSource(controller, u8"LeftMenuButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"RightMenuButton");
	
	inputManagerInstance->RegisterActionInputSource(controller, u8"LeftHatButton");
	inputManagerInstance->RegisterActionInputSource(controller, u8"RightHatButton");
	
	inputManagerInstance->RegisterLinearInputSource(controller, u8"LeftTrigger");
	inputManagerInstance->RegisterLinearInputSource(controller, u8"RightTrigger");
}

using namespace GTSL;

void GameApplication::onWindowResize(const Extent2D extent)
{
	GTSL::StaticVector<TaskDependency, 10> taskDependencies = { { u8"RenderSystem", AccessTypes::READ_WRITE } };

	auto ext = extent;

	auto resize = [](TaskInfo info, Extent2D newSize) {
		auto* renderSystem = info.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");

		renderSystem->OnResize(newSize);
	};
	
	if (extent != 0 && extent != oldSize) {
		gameInstance->AddDynamicTask(u8"windowResize", Delegate<void(TaskInfo, Extent2D)>::Create(resize), taskDependencies, u8"FrameStart", u8"RenderStart", MoveRef(ext));
		oldSize = extent;
	}
}

void GameApplication::keyboardEvent(const Window::KeyboardKeys key, const bool state, bool isFirstkeyOfType) {
	Id id;
	
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

void GameApplication::windowUpdateFunction(void* userData, GTSL::Window::WindowEvents event, void* eventData)
{
	auto* app = static_cast<GameApplication*>(userData);

	switch (event)
	{
	case Window::WindowEvents::FOCUS: {
		auto* focusEventData = static_cast<GTSL::Window::FocusEventData*>(eventData);
		if(focusEventData->Focus) {
			app->gameInstance->DispatchEvent(u8"Application", EventHandle<bool>(u8"OnFocusGain"), GTSL::MoveRef(focusEventData->HadFocus));
		} else {
			app->gameInstance->DispatchEvent(u8"Application", EventHandle<bool>(u8"OnFocusLoss"), GTSL::MoveRef(focusEventData->HadFocus));
		}
		break;
	}
	case GTSL::Window::WindowEvents::CLOSE: app->Close(CloseMode::OK, MakeRange(u8"")); break;
	case GTSL::Window::WindowEvents::KEYBOARD_KEY: {
		auto* keyboardEventData = static_cast<GTSL::Window::KeyboardKeyEventData*>(eventData);
		app->keyboardEvent(keyboardEventData->Key, keyboardEventData->State, keyboardEventData->IsFirstTime);
		break;
	}
	case GTSL::Window::WindowEvents::CHAR: app->GetInputManager()->RecordInputSource(app->keyboard, u8"Character", *static_cast<GTSL::Window::CharEventData*>(eventData)); break;
	case GTSL::Window::WindowEvents::SIZE: {
		auto* sizingEventData = static_cast<GTSL::Window::WindowSizeEventData*>(eventData);
		app->onWindowResize(*sizingEventData);
		break;
	}
	case GTSL::Window::WindowEvents::MOVING: break;
	case GTSL::Window::WindowEvents::MOUSE_MOVE: {
		auto* mouseMoveEventData = static_cast<GTSL::Window::MouseMoveEventData*>(eventData);
		app->GetInputManager()->RecordInputSource(app->mouse, u8"MouseMove", *mouseMoveEventData);
		break;
	}
	case GTSL::Window::WindowEvents::MOUSE_WHEEL: {
		auto* mouseWheelEventData = static_cast<GTSL::Window::MouseWheelEventData*>(eventData);
		app->GetInputManager()->RecordInputSource(app->mouse, u8"MouseWheel", *mouseWheelEventData);
		break;
	}
	case GTSL::Window::WindowEvents::MOUSE_BUTTON: {
		auto* mouseButtonEventData = static_cast<GTSL::Window::MouseButtonEventData*>(eventData);

		switch (mouseButtonEventData->Button)
		{
		case GTSL::Window::MouseButton::LEFT_BUTTON: app->GetInputManager()->RecordInputSource(app->mouse, u8"LeftMouseButton", mouseButtonEventData->State);	break;
		case GTSL::Window::MouseButton::RIGHT_BUTTON: app->GetInputManager()->RecordInputSource(app->mouse, u8"RightMouseButton", mouseButtonEventData->State); break;
		case GTSL::Window::MouseButton::MIDDLE_BUTTON: app->GetInputManager()->RecordInputSource(app->mouse, u8"MiddleMouseButton", mouseButtonEventData->State); break;
		default:;
		}
		break;
	}
	case Window::WindowEvents::DEVICE_CHANGE: {
		BE_LOG_MESSAGE("Device changed!")
		break;
	}
	default:;
	}
}
