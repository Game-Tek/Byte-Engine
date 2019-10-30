#pragma once

#include "Containers/FVector.hpp"
#include "Utility/Functor.h"

template<typename RET, typename... PARAMS>
class Dispatcher
{
	using FunctorType = Functor<RET(PARAMS)>;
	
	FVector<FunctorType> Delegates;
	
public:
	void Subscribe(const FunctorType& _FT) { Delegates.emplace_back(_FT); }
	void Unsubcribe(const FunctorType& _FT) { Delegates.pop(Delegates.find(_FT)); }

	void Dispatch(PARAMS _A)
	{
		for (auto& e : Delegates)
		{
			e(_A);
		}
	}
};
