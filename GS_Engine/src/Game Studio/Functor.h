#pragma once

#include "Event.h"

template<class C, typename ET>
struct Functor
{
	C * Obj = nullptr;
	void (C::*MFPTR)(const ET * Ev) = nullptr;
	
	Functor() = default;

	Functor(C * Obj, void (C::*MFPTR)(const ET * Ev)) : Obj(Obj), MFPTR(MFPTR)
	{
	}

	INLINE void operator()(const ET * Ev) const
	{
		((Obj)->*(MFPTR))(Ev);
	}
};