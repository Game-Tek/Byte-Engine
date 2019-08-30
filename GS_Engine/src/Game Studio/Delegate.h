#pragma once
#include "Core.h"
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

template<typename T>
class delegate_base;

template<typename RET, typename ...PARAMS>
GS_CLASS delegate_base<RET(PARAMS...)>
{
protected:
	using FunctionPointerType = RET(*)(void* this_ptr, PARAMS...);

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

template <typename T> class Delegate;
template <typename T> class multicast_delegate;

template<typename RET, typename... PARAMS>
GS_CLASS Delegate<RET(PARAMS...)> final : delegate_base<RET(PARAMS...)>
{
	friend class multicast_delegate<RET(PARAMS...)>;
	typename delegate_base<RET(PARAMS...)>::InvocationElement invocation;

public:
	Delegate() = default;

	[[nodiscard]] bool isNull() const
	{
		return invocation.FunctionPointer == nullptr;
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
		another.invocation.Clone(invocation);
	}

	template <typename LAMBDA>
	Delegate(const LAMBDA& lambda)
	{
		assign((void*)(&lambda), lambda_stub<LAMBDA>);
	}

	Delegate& operator =(const Delegate& another)
	{
		another.invocation.Clone(invocation);
		return *this;
	}

	template <typename LAMBDA> // template instantiation is not needed, will be deduced (inferred):
	Delegate& operator =(const LAMBDA& instance)
	{
		assign((void*)(&instance), lambda_stub<LAMBDA>);
		return *this;
	}

	bool operator == (const Delegate& another) const
	{
		return invocation == another.invocation;
	}

	bool operator != (const Delegate& another) const
	{
		return invocation != another.invocation;
	}

	bool operator ==(const multicast_delegate<RET(PARAMS...)>& another) const
	{
		return another == (*this);
	}

	bool operator !=(const multicast_delegate<RET(PARAMS...)>& another) const
	{
		return another != (*this);
	}

	template <class T, RET(T::* TMethod)(PARAMS...)>
	static Delegate Create(T* instance)
	{
		return Delegate(instance, method_stub<T, TMethod>);
	}

	template <class T, RET(T::* TMethod)(PARAMS...) const>
	static Delegate Create(T const* instance)
	{
		return Delegate(const_cast<T*>(instance), const_method_stub<T, TMethod>);
	}

	template <RET(*TMethod)(PARAMS...)>
	static Delegate Create()
	{
		return Delegate(nullptr, function_stub<TMethod>);
	}

	template <typename LAMBDA>
	static Delegate Create(const LAMBDA& instance)
	{
		return Delegate((void*)(&instance), lambda_stub<LAMBDA>);
	}

	RET operator()(PARAMS... arg) const
	{
		return (*invocation.FunctionPointer)(invocation.Callee, arg...);
	}

private:

	Delegate(void* anObject, typename delegate_base<RET(PARAMS...)>::FunctionPointerType aStub)
	{
		invocation.Callee = anObject;
		invocation.FunctionPointer = aStub;
	}

	void assign(void* anObject, typename delegate_base<RET(PARAMS...)>::FunctionPointerType aStub)
	{
		this->invocation.Callee = anObject;
		this->invocation.FunctionPointer = aStub;
	}

	template <class T, RET(T::* TMethod)(PARAMS...)>
	static RET method_stub(void* this_ptr, PARAMS... params)
	{
		T* p = static_cast<T*>(this_ptr);
		return (p->*TMethod)(params...);
	}

	template <class T, RET(T::* TMethod)(PARAMS...) const>
	static RET const_method_stub(void* this_ptr, PARAMS... params)
	{
		T* const p = static_cast<T*>(this_ptr);
		return (p->*TMethod)(params...);
	}

	template <RET(*TMethod)(PARAMS...)>
	static RET function_stub(void* this_ptr, PARAMS... params)
	{
		return (TMethod)(params...);
	}

	template <typename LAMBDA>
	static RET lambda_stub(void* this_ptr, PARAMS... arg)
	{
		LAMBDA* p = static_cast<LAMBDA*>(this_ptr);
		return (p->operator())(arg...);
	}
};