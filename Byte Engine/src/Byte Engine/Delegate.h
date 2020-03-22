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
class DelegateBase;

template <typename RET, typename ...PARAMS>
class DelegateBase<RET(PARAMS ...)>
{
protected:
	using FunctionPointerType = RET(*)(void* this_ptr, PARAMS ...);

	struct InvocationElement
	{
		void* Callee = nullptr;
		FunctionPointerType FunctionPointer = nullptr;

		InvocationElement() = default;

		InvocationElement(void* this_ptr, FunctionPointerType aStub) : Callee(this_ptr), FunctionPointer(aStub)
		{
		}

		void Clone(InvocationElement& target) const
		{
			target.FunctionPointer = FunctionPointer;
			target.Callee = Callee;
		}

		bool operator ==(const InvocationElement& another) const
		{
			return another.FunctionPointer == FunctionPointer && another.Callee == Callee;
		}

		bool operator !=(const InvocationElement& another) const
		{
			return another.FunctionPointer != FunctionPointer || another.Callee != Callee;
		}
	};
};

//template <typename T>
//class Delegate;

template <typename RET, typename... PARAMS>
class Delegate<RET(PARAMS ...)> final : DelegateBase<RET(PARAMS ...)>
{
	typename DelegateBase<RET(PARAMS ...)>::InvocationElement functionPointer;

public:
	Delegate() = default;

	[[nodiscard]] bool isNull() const
	{
		return functionPointer.FunctionPointer == nullptr;
	}

	bool operator ==(void* ptr) const
	{
		return (ptr == nullptr) && this->isNull();
	}

	bool operator !=(void* ptr) const
	{
		return (ptr != nullptr) || (!this->isNull());
	}

	Delegate(const Delegate& another)
	{
		another.functionPointer.Clone(functionPointer);
	}

	template <typename LAMBDA>
	Delegate(const LAMBDA& lambda)
	{
		assign(static_cast<void*>(&lambda), lambda_stub<LAMBDA>);
	}

	Delegate& operator =(const Delegate& another)
	{
		another.functionPointer.Clone(functionPointer);
		return *this;
	}

	template <typename LAMBDA> // template instantiation is not needed, will be deduced (inferred):
	Delegate& operator =(const LAMBDA& instance)
	{
		assign(static_cast<void*>(&instance), lambda_stub<LAMBDA>);
		return *this;
	}

	bool operator ==(const Delegate& another) const
	{
		return functionPointer == another.functionPointer;
	}

	bool operator !=(const Delegate& another) const
	{
		return functionPointer != another.functionPointer;
	}

	template <class T, RET(T::* TMethod)(PARAMS ...)>
	static Delegate Create(T* instance)
	{
		return Delegate(instance, method_stub<T, TMethod>);
	}

	template <class T, RET(T::* TMethod)(PARAMS ...) const>
	static Delegate Create(T const* instance)
	{
		return Delegate(const_cast<T*>(instance), const_method_stub<T, TMethod>);
	}

	template <RET(*TMethod)(PARAMS ...)>
	static Delegate Create()
	{
		return Delegate(nullptr, function_stub<TMethod>);
	}

	template <typename LAMBDA>
	static Delegate Create(const LAMBDA& instance)
	{
		return Delegate(static_cast<void*>(&instance), lambda_stub<LAMBDA>);
	}

	RET operator()(PARAMS ... arg) const
	{
		return (*functionPointer.FunctionPointer)(functionPointer.Callee, arg...);
	}

private:

	Delegate(void* anObject, typename DelegateBase<RET(PARAMS ...)>::FunctionPointerType aStub)
	{
		functionPointer.Callee = anObject;
		functionPointer.FunctionPointer = aStub;
	}

	void assign(void* anObject, typename DelegateBase<RET(PARAMS ...)>::FunctionPointerType aStub)
	{
		this->functionPointer.Callee = anObject;
		this->functionPointer.FunctionPointer = aStub;
	}

	template <class T, RET(T::* TMethod)(PARAMS ...)>
	static RET method_stub(void* this_ptr, PARAMS ... params)
	{
		T* p = static_cast<T*>(this_ptr);
		return (p->*TMethod)(params...);
	}

	template <class T, RET(T::* TMethod)(PARAMS ...) const>
	static RET const_method_stub(void* this_ptr, PARAMS ... params)
	{
		T* const p = static_cast<T*>(this_ptr);
		return (p->*TMethod)(params...);
	}

	template <RET(*TMethod)(PARAMS ...)>
	static RET function_stub(void* this_ptr, PARAMS ... params)
	{
		return (TMethod)(params...);
	}

	template <typename LAMBDA>
	static RET lambda_stub(void* this_ptr, PARAMS ... arg)
	{
		LAMBDA* p = static_cast<LAMBDA*>(this_ptr);
		return (p->operator())(arg...);
	}
};

#define MAKE_EVENT(ret, name, ...)  ret On##name(__VA_ARGS__);\
									Functor<ret(__VA_ARGS__)> DelOn##name;\
									Functor<ret(__VA_ARGS__)>& GetOn##nameDelegate() { return DelOn##name; }
