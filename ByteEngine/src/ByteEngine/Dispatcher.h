#pragma once

#include <GTSL/Vector.hpp>
#include "Utility/Delegate.h"

template <typename FT, typename... PARAMS>
class Dispatcher
{
	using FunctorType = Delegate<FT(PARAMS)>;

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
