#include "RenderSystem.h"

#include <GTSL/Window.h>
#include <Windows.h>

#include "ByteEngine/Application/Application.h"

void RenderSystem::InitializeRenderer(const InitializeRendererInfo& initializeRenderer)
{
	GAL::RenderDevice::CreateInfo createinfo;
	createinfo.ApplicationName = GTSL::StaticString<128>("Test");
	GTSL::Array<GAL::Queue::CreateInfo, 1> queue_create_infos(1);
	queue_create_infos[0].Capabilities = static_cast<uint8>(GAL::QueueCapabilities::GRAPHICS);
	queue_create_infos[0].QueuePriority = 1.0f;
	createinfo.QueueCreateInfos = queue_create_infos;
	GTSL::Array<GAL::Queue*, 1> queues;
	queues.EmplaceBack(&graphicsQueue);
	createinfo.Queues = queues;
	::new(&renderDevice) RenderDevice(createinfo);

	GAL::RenderContext::CreateInfo render_context_create_info;
	render_context_create_info.RenderDevice = &renderDevice;
	render_context_create_info.DesiredFramesInFlight = 2;
	render_context_create_info.PresentMode = GAL::PresentMode::FIFO;
	GTSL::Extent2D window_extent;
	initializeRenderer.Window->GetFramebufferExtent(window_extent);
	render_context_create_info.SurfaceArea = window_extent;
	GAL::WindowsWindowData window_data;
	GTSL::Window::Win32NativeHandles native_handles;
	initializeRenderer.Window->GetNativeHandles(&native_handles);
	window_data.WindowHandle = native_handles.HWND;
	window_data.InstanceHandle = GetModuleHandleA(nullptr);
	render_context_create_info.SystemData = &window_data;
	::new(&renderContext) RenderContext(render_context_create_info);
}

void RenderSystem::Initialize(const InitializeInfo& initializeInfo)
{
	GTSL::Array<GTSL::Id64, 8> actsOn{ "RenderSystem" };
	initializeInfo.GameInstance->AddTask("Test", GameInstance::AccessType::READ, GTSL::Delegate<void(const GameInstance::TaskInfo&)>::Create<RenderSystem, &RenderSystem::test>(this), actsOn, "Frame");
}

void RenderSystem::Shutdown()
{
	renderContext.Destroy(&renderDevice);
}

void RenderSystem::test(const GameInstance::TaskInfo& taskInfo)
{
	BE_LOG_SUCCESS("Test task was fired!")
}
