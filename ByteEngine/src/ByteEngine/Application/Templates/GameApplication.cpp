#include "GameApplication.h"

#include "ByteEngine/Application/InputManager.h"

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

	inputManagerInstance->Register2DInputSource("MouseMove");
	
	auto mouse = [](const GTSL::Vector2 a)
	{
		Get()->GetInputManager()->Record2DInputSource("MouseMove", a);
		//BE_BASIC_LOG_MESSAGE("Mouse was moved");
	};

	window.SetOnMouseMoveDelegate(GTSL::Delegate<void(GTSL::Vector2)>::Create(mouse));

}

void GameApplication::OnNormalUpdate()
{
	//std::cout << "Game Application loop update\n";
	systemApplication.UpdateWindow(&window);
}
