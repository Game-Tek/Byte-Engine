#pragma once

#include "Application.h"
#include "ByteEngine/Core.h"

#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Application/InputManager.h"

class WindowSystem : public BE::System {
public:
	WindowSystem(const InitializeInfo& initialize_info) : BE::System(initialize_info, u8"WindowSystem"), WindowTypeIndentifier(initialize_info.ApplicationManager->RegisterType(this, u8"Window")), OnWindowResizeEventHandle(u8"OnWindowResize") {
	}

	DECLARE_BE_TYPE(Window);

	DECLARE_BE_EVENT(OnWindowResize, WindowHandle, GTSL::Extent2D);

#undef CreateWindow

	WindowHandle CreateWindow(const GTSL::StringView id_name, const GTSL::StringView display_name, const GTSL::Extent2D window_extent) {
		uint32 index = windows.GetLength();
		auto& window = windows.EmplaceBack();
		window.window.BindToOS(display_name, window_extent, this, GTSL::Delegate<void(void*, GTSL::Window::WindowEvents, void*)>::Create<WindowSystem, &WindowSystem::windowUpdateFunction>(this));

		window.window.AddDevice(GTSL::Window::DeviceType::MOUSE);
		window.window.AddDevice(GTSL::Window::DeviceType::GAMEPAD);

		window.window.SetWindowVisibility(true);

		return GetApplicationManager()->MakeHandle<WindowHandle>(WindowTypeIndentifier, index);
	}

	GTSL::Vector2 GetWindowPosition() const {
		return windows[0].position;
	}

	GTSL::Extent2D GetWindowClientExtent() const {
		return windows[0].window.GetFramebufferExtent();
	}

	GTSL::Extent2D GetWindowClientExtent(const WindowHandle window_handle) const {
		return windows[window_handle()].window.GetFramebufferExtent();
	}

	const GTSL::Window& GetWindow() const {
		return windows[0].window;
	}

	void Update() {
		for(auto& window : windows) {
			window.window.Update(this, GTSL::Delegate<void(void*, GTSL::Window::WindowEvents, void*)>::Create<WindowSystem, &WindowSystem::windowUpdateFunction>(this));
		}
	}

	InputDeviceHandle keyboard, mouse;
private:
	struct WindowData {
		GTSL::Window window;
		GTSL::Vector2 position;
		WindowHandle windowHandle;
	};
	GTSL::StaticVector<WindowData, 16> windows;

	void windowUpdateFunction(void* userData, GTSL::Window::WindowEvents event, void* eventData) {
		auto* app = static_cast<WindowSystem*>(userData);

		auto* inputManager = BE::Application::Get()->GetInputManager();

		switch (event) {
		case GTSL::Window::WindowEvents::FOCUS: {
			auto* focusEventData = static_cast<GTSL::Window::FocusEventData*>(eventData);
			if(focusEventData->Focus) {
				//app->GetApplicationManager()->DispatchEvent(u8"Application", EventHandle<bool>(u8"OnFocusGain"), GTSL::MoveRef(focusEventData->HadFocus));
			} else {
				//app->GetApplicationManager()->DispatchEvent(u8"Application", EventHandle<bool>(u8"OnFocusLoss"), GTSL::MoveRef(focusEventData->HadFocus));
			}
			break;
		}
		case GTSL::Window::WindowEvents::CLOSE: BE::Application::Get()->Close(BE::Application::CloseMode::OK, u8"User closed window."); break;
		case GTSL::Window::WindowEvents::KEYBOARD_KEY: {
			auto* keyboardEventData = static_cast<GTSL::Window::KeyboardKeyEventData*>(eventData);

			auto keyboardEvent = [&](const GTSL::Window::KeyboardKeys key, const bool state, bool isFirstkeyOfType) {
				GTSL::StaticString<64> id;
				
				switch (key) {
				case GTSL::Window::KeyboardKeys::Q: id = u8"Q_Key"; break; case GTSL::Window::KeyboardKeys::W: id = u8"W_Key"; break;
				case GTSL::Window::KeyboardKeys::E: id = u8"E_Key"; break; case GTSL::Window::KeyboardKeys::R: id = u8"R_Key"; break;
				case GTSL::Window::KeyboardKeys::T: id = u8"T_Key"; break; case GTSL::Window::KeyboardKeys::Y: id = u8"Y_Key"; break;
				case GTSL::Window::KeyboardKeys::U: id = u8"U_Key"; break; case GTSL::Window::KeyboardKeys::I: id = u8"I_Key"; break;
				case GTSL::Window::KeyboardKeys::O: id = u8"O_Key"; break; case GTSL::Window::KeyboardKeys::P: id = u8"P_Key"; break;
				case GTSL::Window::KeyboardKeys::A: id = u8"A_Key"; break; case GTSL::Window::KeyboardKeys::S: id = u8"S_Key"; break;
				case GTSL::Window::KeyboardKeys::D: id = u8"D_Key"; break; case GTSL::Window::KeyboardKeys::F: id = u8"F_Key"; break;
				case GTSL::Window::KeyboardKeys::G: id = u8"G_Key"; break; case GTSL::Window::KeyboardKeys::H: id = u8"H_Key"; break;
				case GTSL::Window::KeyboardKeys::J: id = u8"J_Key"; break; case GTSL::Window::KeyboardKeys::K: id = u8"K_Key"; break;
				case GTSL::Window::KeyboardKeys::L: id = u8"L_Key"; break; case GTSL::Window::KeyboardKeys::Z: id = u8"Z_Key"; break;
				case GTSL::Window::KeyboardKeys::X: id = u8"X_Key"; break; case GTSL::Window::KeyboardKeys::C: id = u8"C_Key"; break;
				case GTSL::Window::KeyboardKeys::V: id = u8"V_Key"; break; case GTSL::Window::KeyboardKeys::B: id = u8"B_Key"; break;
				case GTSL::Window::KeyboardKeys::N: id = u8"N_Key"; break; case GTSL::Window::KeyboardKeys::M: id = u8"M_Key"; break;
				case GTSL::Window::KeyboardKeys::Keyboard0: id = u8"0_Key"; break; case GTSL::Window::KeyboardKeys::Keyboard1: id = u8"1_Key"; break;
				case GTSL::Window::KeyboardKeys::Keyboard2: id = u8"2_Key"; break; case GTSL::Window::KeyboardKeys::Keyboard3: id = u8"3_Key"; break;
				case GTSL::Window::KeyboardKeys::Keyboard4: id = u8"4_Key"; break; case GTSL::Window::KeyboardKeys::Keyboard5: id = u8"5_Key"; break;
				case GTSL::Window::KeyboardKeys::Keyboard6: id = u8"6_Key"; break; case GTSL::Window::KeyboardKeys::Keyboard7: id = u8"7_Key"; break;
				case GTSL::Window::KeyboardKeys::Keyboard8: id = u8"8_Key"; break; case GTSL::Window::KeyboardKeys::Keyboard9: id = u8"9_Key"; break;
				case GTSL::Window::KeyboardKeys::Backspace: id = u8"Backspace_Key"; break;
				case GTSL::Window::KeyboardKeys::Enter: id = u8"Enter_Key"; break;
				case GTSL::Window::KeyboardKeys::Supr: id = u8"Supr_Key"; break;
				case GTSL::Window::KeyboardKeys::Tab: id = u8"Tab_Key"; break;
				case GTSL::Window::KeyboardKeys::CapsLock: id = u8"CapsLock_Key"; break;
				case GTSL::Window::KeyboardKeys::Esc: id = u8"Esc_Key"; break;
				case GTSL::Window::KeyboardKeys::RShift: id = u8"RightShift_Key"; break; case GTSL::Window::KeyboardKeys::LShift: id = u8"LeftShift_Key"; break;
				case GTSL::Window::KeyboardKeys::RControl: id = u8"RightControl_Key"; break; case GTSL::Window::KeyboardKeys::LControl: id = u8"LeftControl_Key"; break;
				case GTSL::Window::KeyboardKeys::Alt: id = u8"LeftAlt_Key"; break; case GTSL::Window::KeyboardKeys::AltGr: id = u8"RightAlt_Key"; break;
				case GTSL::Window::KeyboardKeys::UpArrow: id = u8"Up_Key"; break; case GTSL::Window::KeyboardKeys::RightArrow: id = u8"Right_Key"; break;
				case GTSL::Window::KeyboardKeys::DownArrow: id = u8"Down_Key"; break; case GTSL::Window::KeyboardKeys::LeftArrow: id = u8"Left_Key"; break;
				case GTSL::Window::KeyboardKeys::SpaceBar: id = u8"SpaceBar_Key"; break;
				case GTSL::Window::KeyboardKeys::Numpad0: id = u8"Numpad0_Key"; break; case GTSL::Window::KeyboardKeys::Numpad1: id = u8"Numpad1_Key"; break;
				case GTSL::Window::KeyboardKeys::Numpad2: id = u8"Numpad2_Key"; break; case GTSL::Window::KeyboardKeys::Numpad3: id = u8"Numpad3_Key"; break;
				case GTSL::Window::KeyboardKeys::Numpad4: id = u8"Numpad4_Key"; break; case GTSL::Window::KeyboardKeys::Numpad5: id = u8"Numpad5_Key"; break;
				case GTSL::Window::KeyboardKeys::Numpad6: id = u8"Numpad6_Key"; break; case GTSL::Window::KeyboardKeys::Numpad7: id = u8"Numpad7_Key"; break;
				case GTSL::Window::KeyboardKeys::Numpad8: id = u8"Numpad8_Key"; break; case GTSL::Window::KeyboardKeys::Numpad9: id = u8"Numpad9_Key"; break;
				case GTSL::Window::KeyboardKeys::F1: id = u8"F1_Key"; break; case GTSL::Window::KeyboardKeys::F2: id = u8"F2_Key"; break;
				case GTSL::Window::KeyboardKeys::F3: id = u8"F3_Key"; break; case GTSL::Window::KeyboardKeys::F4: id = u8"F4_Key"; break;
				case GTSL::Window::KeyboardKeys::F5: id = u8"F5_Key"; break; case GTSL::Window::KeyboardKeys::F6: id = u8"F6_Key"; break;
				case GTSL::Window::KeyboardKeys::F7: id = u8"F7_Key"; break; case GTSL::Window::KeyboardKeys::F8: id = u8"F8_Key"; break;
				case GTSL::Window::KeyboardKeys::F9: id = u8"F9_Key"; break; case GTSL::Window::KeyboardKeys::F10: id = u8"F10_Key"; break;
				case GTSL::Window::KeyboardKeys::F11: id = u8"F11_Key"; break; case GTSL::Window::KeyboardKeys::F12: id = u8"F12_Key"; break;
				default: break;
				}
			
				if (isFirstkeyOfType) {
					inputManager->RecordInputSource(keyboard, id, state);
				}
			};

			keyboardEvent(keyboardEventData->Key, keyboardEventData->State, keyboardEventData->IsFirstTime);
			break;
		}
		case GTSL::Window::WindowEvents::CHAR: inputManager->RecordInputSource(app->keyboard, u8"Character", static_cast<char32_t>(*static_cast<GTSL::Window::CharEventData*> (eventData))); break;
		case GTSL::Window::WindowEvents::SIZE: {
			auto* sizingEventData = static_cast<GTSL::Window::WindowSizeEventData*>(eventData);
			app->GetApplicationManager()->DispatchEvent(this, GetOnWindowResizeEventHandle(), GTSL::MoveRef(app->windows[0].windowHandle), GTSL::MoveRef(*sizingEventData));
			break;
		}
		case GTSL::Window::WindowEvents::MOVING: {
			auto* moveData = static_cast<GTSL::Window::WindowMoveEventData*>(eventData);

			windows.front().position.X() = moveData->X;
			windows.front().position.Y() = moveData->Y;

			break;
		}
		case GTSL::Window::WindowEvents::MOUSE_MOVE: {
			auto* mouseMoveEventData = static_cast<GTSL::Window::MouseMoveEventData*>(eventData);
			inputManager->RecordInputSource(app->mouse, u8"MouseMove", *mouseMoveEventData);
			break;
		}
		case GTSL::Window::WindowEvents::MOUSE_WHEEL: {
			auto* mouseWheelEventData = static_cast<GTSL::Window::MouseWheelEventData*>(eventData);
			inputManager->RecordInputSource(app->mouse, u8"MouseWheel", *mouseWheelEventData);
			break;
		}
		case GTSL::Window::WindowEvents::MOUSE_BUTTON: {
			auto* mouseButtonEventData = static_cast<GTSL::Window::MouseButtonEventData*>(eventData);

			switch (mouseButtonEventData->Button) {
			case GTSL::Window::MouseButton::LEFT_BUTTON: inputManager->RecordInputSource(app->mouse, u8"LeftMouseButton", mouseButtonEventData->State);	break;
			case GTSL::Window::MouseButton::RIGHT_BUTTON: inputManager->RecordInputSource(app->mouse, u8"RightMouseButton", mouseButtonEventData->State); break;
			case GTSL::Window::MouseButton::MIDDLE_BUTTON: inputManager->RecordInputSource(app->mouse, u8"MiddleMouseButton", mouseButtonEventData->State); break;
			default:;
			}
			break;
		}
		case GTSL::Window::WindowEvents::DEVICE_CHANGE: {
			BE_LOG_MESSAGE(u8"Device changed!")
			break;
		}
		default:;
		}
	}
};