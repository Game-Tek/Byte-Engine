#include "JobSystem.h"

JobSystem::JobSystem()
{
	auto thread_count = std::thread::hardware_concurrency();

	threads.resize(thread_count - 1);
}
