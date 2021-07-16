#pragma once

#include <GAL/Vulkan/VulkanPipelines.h>

#include "ByteEngine/Core.h"
#include "ByteEngine/Object.h"

#include "ByteEngine/Debug/Logger.h"

#include <GTSL/Array.hpp>
#include <GTSL/Atomic.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Semaphore.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/BlockingQueue.h>
#include <GTSL/Thread.h>
#include <GTSL/Tuple.h>

//https://github.com/mvorbrodt/blog

class ThreadPool : public Object
{
	using TaskDelegate = GTSL::Delegate<void(ThreadPool*, GTSL::SmartPointer<void*, BE::PAR>)>;
	struct Task {
		TaskDelegate Delegate;
		GTSL::SmartPointer<void*, BE::PAR> TaskInfo;

		Task(const BE::PAR& a) : TaskInfo(a) {}
		Task(TaskDelegate del, GTSL::SmartPointer<void*, BE::PAR>&& info) : Delegate(del),  TaskInfo(MoveRef(info)) {}
	};
public:
	explicit ThreadPool() : Object(u8"Thread Pool")
	{		
		//lambda
		auto workers_loop = [](ThreadPool* pool, const uint8 i) {			
			while (true) {		
				Task task(pool->GetPersistentAllocator());
				
				for (auto n = 0; n < threadCount * K; ++n) {
					auto queueIndex = (i + n) % threadCount;

					if (pool->queues[queueIndex].TryPop(task)) {
						task.Delegate(pool, GTSL::MoveRef(task.TaskInfo));
						pool->queues[queueIndex].Done();
						break;
					}
				}

				//if (!GTSL::Get<TUPLE_LAMBDA_DELEGATE_INDEX>(task) && !pool->queues[i].Pop(task)) { break;	}
				if (pool->queues[i].Pop(task)) {
					task.Delegate(pool, GTSL::MoveRef(task.TaskInfo));
					pool->queues[i].Done();
				} else {
					break;
				}
			}
		};

		for (uint8 i = 0; i < threadCount; ++i) { //initialize all queues first, as threads try to access ALL queues on initialization
			queues.EmplaceBack(); //don't remove we need to force initialization of blocking queues
		}
		
		for (uint8 i = 0; i < threadCount; ++i) {
			//Constructing threads with function and I parameter. i + 1 is because we leave id 0 to the main thread
			threads.EmplaceBack(GetPersistentAllocator(), i + 1, GTSL::Delegate<void(ThreadPool*, uint8)>::Create(workers_loop), this, i);
			threads[i].SetPriority(GTSL::Thread::Priority::HIGH);
		}
	}

	~ThreadPool()
	{
		for (auto& queue : queues) { queue.End(); }
		for (auto& thread : threads) { thread.Join(GetPersistentAllocator()); }
	}

	template<typename F, typename... ARGS>
	void EnqueueTask(const GTSL::Delegate<F>& task, ARGS&&... args) {
		const auto currentIndex = index++;
		
		GTSL::SmartPointer<TaskInfo<F, ARGS...>, BE::PAR> taskInfoAlloc(GetPersistentAllocator(), currentIndex, task, GTSL::ForwardRef<ARGS>(args)...);

		auto work = [](ThreadPool* threadPool, GTSL::SmartPointer<void*, BE::PAR> voidTask) -> void {
			TaskInfo<F, ARGS...>* taskInfo = reinterpret_cast<TaskInfo<F, ARGS...>*>(voidTask.GetData());

			BE_ASSERT(taskInfo->TimesRun == 0, "")

			++taskInfo->TimesRun;
			
			GTSL::Call(taskInfo->Delegate, GTSL::MoveRef(taskInfo->Arguments));

			//GTSL::Delete<TaskInfo<F, ARGS...>>(&taskInfo, threadPool->GetPersistentAllocator());
		};		

		for (auto n = 0; n < threadCount * K; ++n) {
			//Try to Push work into queues, if success return else when Done looping place into some queue.
		
			if (queues[(currentIndex + n) % threadCount].TryPush(TaskDelegate::Create(work), GTSL::MoveRef(taskInfoAlloc))) { return; }
		}

		queues[currentIndex % threadCount].Push(TaskDelegate::Create(work), GTSL::MoveRef(taskInfoAlloc));
	}

	uint8 GetNumberOfThreads() { return threadCount; }

private:
	inline const static uint8 threadCount{ /*static_cast<uint8>(GTSL::Thread::ThreadCount() - 1)*/ 1 };
	GTSL::Atomic<uint32> index{ 0 }, runTasks{ 0 };
	
	GTSL::Array<GTSL::BlockingQueue<Task>, 32> queues;
	GTSL::Array<GTSL::Thread, 32> threads;

	template<typename T, typename... ARGS>
	struct TaskInfo
	{
		TaskInfo(uint32 i, const GTSL::Delegate<T>& delegate, GTSL::Tuple<ARGS...>&& args) : Delegate(delegate), Arguments(GTSL::MoveRef(args)), Index(i)
		{
		}

		TaskInfo(uint32 i, const GTSL::Delegate<T>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::ForwardRef<ARGS>(args)...), Index(i)
		{
		}
		
		GTSL::Delegate<T> Delegate;
		GTSL::Tuple<ARGS...> Arguments;
		uint32 Index = 0;
		uint32 TimesRun = 0;
	};
	/**
	 * \brief Number of times to loop around the queues to find one that is free.
	 */
	inline static constexpr uint8 K{ 2 };
};