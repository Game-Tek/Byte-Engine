#pragma once

#include "Application.h"

#if (_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <Windows.h>

namespace GTSL
{
	class WindowsApplication final : public Application
	{
		HINSTANCE instance = nullptr;

	public:
		explicit WindowsApplication(const ApplicationCreateInfo& applicationCreateInfo);

		void Update() override;

		void Close() override;

		void GetNativeHandles(void* nativeHandles) override;
		
		[[nodiscard]] HINSTANCE GetInstance() const { return instance; }
	};
}
#endif