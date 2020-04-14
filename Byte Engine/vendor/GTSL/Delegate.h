#pragma once

/*
	Copyright (C) 2017 by Sergey A Kryukov: derived work
	http://www.SAKryukov.org
	http://www.codeproject.com/Members/SAKryukov
	Based on original work by Sergey Ryazanov:
	"The Impossibly Fast C++ Delegates", 18 Jul 2005
	https://www.codeproject.com/articles/11015/the-impossibly-fast-c-delegates
	MIT license:
	http://en.wikipedia.org/wiki/MIT_License
	Original publication: https://www.codeproject.com/Articles/1170503/The-Impossibly-Fast-Cplusplus-Delegates-Fixed
*/

template <typename T>
class Delegate;

template <typename RET, typename... PARAMS>
class Delegate<RET(PARAMS ...)> final
{
	RET(*callerFunction)(void*, PARAMS...) { nullptr };
	void* callee{ nullptr };
	
public:
	typedef decltype(callerFunction) call_signature;
	
	Delegate() = default;
	~Delegate() = default;

	operator bool() const noexcept { return callerFunction; }

	template <typename LAMBDA>
	Delegate(LAMBDA& lambda) : callerFunction(&lambdaCaller<LAMBDA>), callee(reinterpret_cast<void*>(&lambda))
	{
	}

	Delegate& operator =(const Delegate& another) = default;

	template <typename LAMBDA> // template instantiation is not needed, will be deduced (inferred):
	Delegate& operator=(const LAMBDA& instance) { assign(static_cast<void*>(&instance), lambdaCaller<LAMBDA>); return *this; }

	bool operator ==(const Delegate& another) const { return callerFunction == another.callerFunction && callee == another.callee; }

	bool operator !=(const Delegate& another) const { return callerFunction != another.callerFunction; }

	template <class T, RET(T::*METHOD)(PARAMS...)>
	static Delegate Create(T* instance) { return Delegate(instance, methodCaller<T, METHOD>); }

	template <class T, RET(T::* CONST_METHOD)(PARAMS...) const>
	static Delegate Create(T const* instance) { return Delegate(const_cast<T*>(instance), constMethodCaller<T, CONST_METHOD>); }

	template <RET(*FUNCTION)(PARAMS ...)>
	static Delegate Create() { return Delegate(nullptr, functionCaller<FUNCTION>); }

	template <typename LAMBDA>
	static Delegate Create(const LAMBDA& instance) { return Delegate(static_cast<void*>(&instance), lambdaCaller<LAMBDA>); }

	RET operator()(PARAMS... arg) const { return (*callerFunction)(callee, arg...); }

private:
	
	template <class T, RET(T::*METHOD)(PARAMS ...)>
	static RET methodCaller(void* callee, PARAMS... params) { return (static_cast<T*>(callee)->*METHOD)(params...); }

	template <class T, RET(T:: *CONST_METHOD)(PARAMS ...) const>
	static RET constMethodCaller(void* callee, PARAMS... params) { return (static_cast<const T*>(callee)->*CONST_METHOD)(params...); }

	template <RET(*FUNCTION)(PARAMS ...)>
	static RET functionCaller(void* callee, PARAMS... params) { return (FUNCTION)(params...); }

	template <typename LAMBDA>
	static RET lambdaCaller(void* callee, PARAMS... arg) { return (static_cast<LAMBDA*>(callee)->operator())(arg...); }
};