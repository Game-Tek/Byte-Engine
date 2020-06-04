#include "GameApplication.h"

#include "ByteEngine/Application/InputManager.h"

#include <GTSL/Input.h>

void GameApplication::Init()
{
	Application::Init();

	BE::TransientAllocatorReference transient_allocator_reference("Application");

	GTSL::Window::WindowCreateInfo create_window_info;
	create_window_info.Application = &systemApplication;
	create_window_info.Name = GTSL::StaticString<1024>(GetApplicationName());
	create_window_info.Extent = {1280, 720};
	::new(&window) GTSL::Window(create_window_info);

	//window.SetOnResizeDelegate(Delegate<void(const GTSL::Extent2D&)>::Create<GameApplication, &GameApplication::resize>(this));

	auto window_resize = [](const GTSL::Extent2D& a)
	{
	};
	window.SetOnWindowResizeDelegate(GTSL::Delegate<void(const GTSL::Extent2D&)>::Create(window_resize));

	auto window_close = []()
	{
		Get()->PromptClose();
		Get()->Close(CloseMode::OK, nullptr);
	};
	window.SetOnCloseDelegate(GTSL::Delegate<void()>::Create(window_close));

	auto window_move = [](uint16 x, uint16 y)
	{

	};
	window.SetOnWindowMoveDelegate(GTSL::Delegate<void(uint16, uint16)>::Create(window_move));
	
	window.ShowWindow();

	SetupInputSources();
}

void GameApplication::OnNormalUpdate()
{
	//std::cout << "Game Application loop update\n";
	systemApplication.UpdateWindow(&window);
}

void GameApplication::SetupInputSources()
{
	RegisterMouse();
	RegisterKeyboard();
}

void GameApplication::RegisterMouse()
{

	inputManagerInstance->Register2DInputSource("MouseMove");

	auto mouse_move = [](const GTSL::Vector2 a)
	{
		Get()->GetInputManager()->Record2DInputSource("MouseMove", a);
	};

	inputManagerInstance->RegisterActionInputSource("LeftMouseButton");
	inputManagerInstance->RegisterActionInputSource("RightMouseButton");
	inputManagerInstance->RegisterActionInputSource("MiddleMouseButton");

	auto mouse_click = [](const GTSL::Window::MouseButton button, const GTSL::ButtonState buttonState)
	{
		const bool state = buttonState == GTSL::ButtonState::PRESSED ? true : false;

		switch (button)
		{
		case GTSL::Window::MouseButton::LEFT_BUTTON: Get()->GetInputManager()->RecordActionInputSource("LeftMouseButton", state); break;
		case GTSL::Window::MouseButton::RIGHT_BUTTON: Get()->GetInputManager()->RecordActionInputSource("RightMouseButton", state); break;
		case GTSL::Window::MouseButton::MIDDLE_BUTTON: Get()->GetInputManager()->RecordActionInputSource("MiddleMouseButton", state); break;
		default:;
		}
	};

	inputManagerInstance->RegisterLinearInputSource("MouseWheel");

	auto mouse_wheel = [](const float value)
	{
		Get()->GetInputManager()->RecordLinearInputSource("MouseWheel", value);
	};

	window.SetOnMouseMoveDelegate(GTSL::Delegate<void(GTSL::Vector2)>::Create(mouse_move));
	window.SetOnMouseButtonClickDelegate(GTSL::Delegate<void(GTSL::Window::MouseButton, GTSL::ButtonState)>::Create(mouse_click));
	window.SetOnMouseWheelMoveDelegate(GTSL::Delegate<void(float32)>::Create(mouse_wheel));
}

void GameApplication::RegisterKeyboard()
{
	inputManagerInstance->RegisterCharacterInputSource("Keyboard");
	
	auto char_event = [](const uint32 ch)
	{
		Get()->GetInputManager()->RecordCharacterInputSource("Keyboard", ch);
	};
	window.SetOnCharEventDelegate(GTSL::Delegate<void(uint32)>::Create(char_event));

	inputManagerInstance->RegisterActionInputSource("Q_Key"); inputManagerInstance->RegisterActionInputSource("W_Key");
	inputManagerInstance->RegisterActionInputSource("E_Key"); inputManagerInstance->RegisterActionInputSource("R_Key");
	inputManagerInstance->RegisterActionInputSource("T_Key"); inputManagerInstance->RegisterActionInputSource("Y_Key");
	inputManagerInstance->RegisterActionInputSource("U_Key"); inputManagerInstance->RegisterActionInputSource("I_Key");
	inputManagerInstance->RegisterActionInputSource("O_Key"); inputManagerInstance->RegisterActionInputSource("P_Key");
	inputManagerInstance->RegisterActionInputSource("A_Key"); inputManagerInstance->RegisterActionInputSource("S_Key");
	inputManagerInstance->RegisterActionInputSource("D_Key"); inputManagerInstance->RegisterActionInputSource("F_Key");
	inputManagerInstance->RegisterActionInputSource("G_Key"); inputManagerInstance->RegisterActionInputSource("H_Key");
	inputManagerInstance->RegisterActionInputSource("J_Key"); inputManagerInstance->RegisterActionInputSource("K_Key");
	inputManagerInstance->RegisterActionInputSource("L_Key"); inputManagerInstance->RegisterActionInputSource("Z_Key");
	inputManagerInstance->RegisterActionInputSource("X_Key"); inputManagerInstance->RegisterActionInputSource("C_Key");
	inputManagerInstance->RegisterActionInputSource("V_Key"); inputManagerInstance->RegisterActionInputSource("B_Key");
	inputManagerInstance->RegisterActionInputSource("N_Key"); inputManagerInstance->RegisterActionInputSource("M_Key");
	inputManagerInstance->RegisterActionInputSource("0_Key"); inputManagerInstance->RegisterActionInputSource("1_Key");
	inputManagerInstance->RegisterActionInputSource("2_Key"); inputManagerInstance->RegisterActionInputSource("3_Key");
	inputManagerInstance->RegisterActionInputSource("4_Key"); inputManagerInstance->RegisterActionInputSource("5_Key");
	inputManagerInstance->RegisterActionInputSource("6_Key"); inputManagerInstance->RegisterActionInputSource("7_Key");
	inputManagerInstance->RegisterActionInputSource("8_Key"); inputManagerInstance->RegisterActionInputSource("9_Key");
	inputManagerInstance->RegisterActionInputSource("Backspace_Key");		inputManagerInstance->RegisterActionInputSource("Enter_Key");
	inputManagerInstance->RegisterActionInputSource("Supr_Key");			inputManagerInstance->RegisterActionInputSource("Tab_Key");
	inputManagerInstance->RegisterActionInputSource("CapsLock_Key");		inputManagerInstance->RegisterActionInputSource("Esc_Key");
	inputManagerInstance->RegisterActionInputSource("RightShift_Key");		inputManagerInstance->RegisterActionInputSource("LeftShift_Key");
	inputManagerInstance->RegisterActionInputSource("RightControl_Key");	inputManagerInstance->RegisterActionInputSource("LeftControl_Key");
	inputManagerInstance->RegisterActionInputSource("RightAlt_Key");		inputManagerInstance->RegisterActionInputSource("LeftAlt_Key");
	inputManagerInstance->RegisterActionInputSource("UpArrow_Key");			inputManagerInstance->RegisterActionInputSource("RightArrow_Key");
	inputManagerInstance->RegisterActionInputSource("DownArrow_Key");		inputManagerInstance->RegisterActionInputSource("LeftArrow_Key");
	inputManagerInstance->RegisterActionInputSource("SpaceBar_Key");
	inputManagerInstance->RegisterActionInputSource("Numpad0_Key"); inputManagerInstance->RegisterActionInputSource("Numpad1_Key");
	inputManagerInstance->RegisterActionInputSource("Numpad2_Key"); inputManagerInstance->RegisterActionInputSource("Numpad3_Key");
	inputManagerInstance->RegisterActionInputSource("Numpad4_Key"); inputManagerInstance->RegisterActionInputSource("Numpad5_Key");
	inputManagerInstance->RegisterActionInputSource("Numpad6_Key"); inputManagerInstance->RegisterActionInputSource("Numpad7_Key");
	inputManagerInstance->RegisterActionInputSource("Numpad8_Key"); inputManagerInstance->RegisterActionInputSource("Numpad9_Key");

	auto key_press = [](const GTSL::Window::KeyboardKeys key, const GTSL::ButtonState state)
	{
		const char* id = nullptr;
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
		case GTSL::Window::KeyboardKeys::F1: break;
		case GTSL::Window::KeyboardKeys::F2: break;
		case GTSL::Window::KeyboardKeys::F3: break;
		case GTSL::Window::KeyboardKeys::F4: break;
		case GTSL::Window::KeyboardKeys::F5: break;
		case GTSL::Window::KeyboardKeys::F6: break;
		case GTSL::Window::KeyboardKeys::F7: break;
		case GTSL::Window::KeyboardKeys::F8: break;
		case GTSL::Window::KeyboardKeys::F9: break;
		case GTSL::Window::KeyboardKeys::F10: break;
		case GTSL::Window::KeyboardKeys::F11: break;
		case GTSL::Window::KeyboardKeys::F12: break;
		default: break;
		}

		bool val = state == GTSL::ButtonState::PRESSED ? true : false;
		
		if(id)
		{
			Get()->GetInputManager()->RecordActionInputSource(id, val);
		}
	};
	
	window.SetOnKeyEventDelegate(GTSL::Delegate<void(GTSL::Window::KeyboardKeys, GTSL::ButtonState)>::Create(key_press));
}
