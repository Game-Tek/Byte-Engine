#pragma once

namespace GTSL
{
	class Application
	{
	public:
		virtual ~Application() = default;

	public:
		struct ApplicationCreateInfo
		{
		};
		explicit Application(const ApplicationCreateInfo& applicationCreateInfo);

		virtual void Update() = 0;

		virtual void Close() = 0;

		struct Win32NativeHandles
		{
			void* HINSTANCE{ nullptr };
		};
		virtual void GetNativeHandles(void* nativeHandles) = 0;
	};
}