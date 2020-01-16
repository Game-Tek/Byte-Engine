#pragma once

#include "Containers/FVector.hpp"
#include "Thread.h"

class JobSystem
{
	FVector<Thread> threads;
	
public:
	JobSystem();
	
	//Job StartJob(const Delegate& function);
};
