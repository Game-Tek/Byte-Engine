#pragma once

#include "Core.h"

#include <thread>

namespace GTSL
{
	class Thread
	{
		std::thread thread;

	public:
		template<typename F, typename... P>
		explicit Thread(F&& lambda, P&&... params) : thread(std::forward<F>(lambda)..., std::forward<P>(params)...)
		{
		}

		static uint32 ThisTreadID() noexcept { return std::hash<std::thread::id>{}(std::this_thread::get_id()); }
		static uint8 ThreadCount() noexcept { return std::thread::hardware_concurrency(); }
		//static Thread ThisThread() { return Thread(std::this_thread::); }

		void Join() { thread.join(); }
		void Detach() { thread.detach(); }

		[[nodiscard]] uint32 GetId() const noexcept { return std::hash<std::thread::id>{}(thread.get_id()); }

		[[nodiscard]] bool CanBeJoined() const noexcept { return thread.joinable(); }
	};
}