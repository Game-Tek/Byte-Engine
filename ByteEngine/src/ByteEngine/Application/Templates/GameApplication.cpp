#include "GameApplication.h"

#include "ByteEngine/Application/InputManager.h"

#include <GTSL/Input.h>

void GameApplication::Init()
{
	Application::Init();

	BE::TransientAllocatorReference transient_allocator_reference("Application");

	GTSL::Window::WindowCreateInfo create_window_info;
	create_window_info.Application = &systemApplication;
	create_window_info.Name = GTSL::String(GetApplicationName(), &transient_allocator_reference);
	create_window_info.Extent = {1280, 720};
	::new(&window) GTSL::Window(create_window_info);

	//window.SetOnResizeDelegate(Delegate<void(const GTSL::Extent2D&)>::Create<GameApplication, &GameApplication::resize>(this));

	auto window_resize = [](const GTSL::Extent2D& a)
	{
	};

	window.SetOnResizeDelegate(GTSL::Delegate<void(const GTSL::Extent2D&)>::Create(window_resize));
	
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
}
