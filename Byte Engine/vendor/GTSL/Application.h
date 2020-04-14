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
	};
}