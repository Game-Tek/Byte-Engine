#pragma once

namespace GAL
{
	class Window;
	class RenderDevice;
	class Queue;
	class RenderTarget;
	
	class Surface
	{
	public:
		Surface() = default;
	};

	class RenderContext
	{
	public:
		RenderContext() = default;

		//explicit RenderContext(const CreateInfo& createInfo);
		
		~RenderContext() = default;
	};
}
