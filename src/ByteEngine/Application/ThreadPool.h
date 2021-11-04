#pragma once

#include "ByteEngine/Core.h"
#include "ByteEngine/Object.h"

#include "ByteEngine/Debug/Logger.h"

#include <GTSL/Vector.hpp>
#include <GTSL/Atomic.hpp>
#include <GTSL/Algorithm.hpp>
#include <GTSL/Semaphore.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/BlockingQueue.h>
#include <GTSL/Thread.hpp>
#include <GTSL/Tuple.hpp>

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
	explicit ThreadPool(const uint8 tCount) : Object(u8"Thread Pool"), threadCount(tCount)
	{		
		//lambda
		auto workers_loop = [](ThreadPool* pool, const uint8 i) {			
			while (true) {		
				Task task(pool->GetPersistentAllocator());
				
				for (auto n = 0; n < pool->threadCount * K; ++n) {
					auto queueIndex = (i + n) % pool->threadCount;

					if (pool->queues[queueIndex].TryPop(task)) {
						task.Delegate(pool, MoveRef(task.TaskInfo));
						pool->queues[queueIndex].Done();
						break;
					}
				}

				//if (!GTSL::Get<TUPLE_LAMBDA_DELEGATE_INDEX>(task) && !pool->queues[i].Pop(task)) { break;	}
				if (pool->queues[i].Pop(task)) {
					task.Delegate(pool, MoveRef(task.TaskInfo));
					pool->queues[i].Done();
				} else {
					break;
				}
			}
		};

		//for (uint8 i = 0; i < threadCount; ++i) { //initialize all queues first, as threads try to access ALL queues on initialization
		//	queues.EmplaceBack(); //don't remove we need to force initialization of blocking queues
		//}
		
		for (uint8 i = 0; i < threadCount; ++i) {
			//Constructing threads with function and I parameter. i + 1 is because we leave id 0 to the main thread
			threads.EmplaceBack(GetPersistentAllocator(), i + 1, GTSL::Delegate<void(ThreadPool*, uint8)>::Create(workers_loop), this, i);
			threads[i].SetPriority(GTSL::Thread::Priority::HIGH);
		}
	}

	ThreadPool(const ThreadPool&) = delete;

	~ThreadPool()
	{
		for (uint32 i = 0; i < threadCount; ++i) { queues[i].End(); }
		for (uint32 i = 0; i < threadCount; ++i) { threads[i].Join(GetPersistentAllocator()); }
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
	const uint8 threadCount = 0;
	GTSL::Atomic<uint32> index{ 0 }, runTasks{ 0 };
	
	std::array<GTSL::BlockingQueue<Task>, 32> queues;
	GTSL::StaticVector<GTSL::Thread, 32> threads;

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