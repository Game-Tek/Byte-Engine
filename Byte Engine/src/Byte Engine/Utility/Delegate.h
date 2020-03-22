#pragma once

template<class T, typename RET, typename... ARGS>
class ClassCallObject final
{
	T* object = nullptr;
	RET(T::* function)(ARGS...);

public:

	ClassCallObject(T* obj, RET(T::* func)(ARGS...)) : object(obj), function(func) {}

	RET operator()(ARGS&&... args) { return (object->*function)(std::forward<ARGS>(args)...); }
};

template<class CalleeType, typename RET, typename... PARAMS >
class Delegate
{
	ClassCallObject<CalleeType, RET, PARAMS...> callObject;

public:
	Delegate(CalleeType* obj, RET(CalleeType::* func)(PARAMS...)) : callObject(ClassCallObject<CalleeType, RET, PARAMS...>(obj, func)) {}

	RET operator()(PARAMS&&... params)
	{
		return callObject(std::forward<PARAMS>(params)...);
	}
};