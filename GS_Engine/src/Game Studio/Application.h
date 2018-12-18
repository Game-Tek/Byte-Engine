#pragma once

#include "Core.h"

namespace GS
{
	GS_CLASS Application
	{
		public:
			Application();

			virtual ~Application();

			void Run();
	};

	Application * CreateApplication();
}