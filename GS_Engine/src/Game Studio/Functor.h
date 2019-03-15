#pragma once

#include "Object.h"

#include "Event.h"

typedef void (*MemberFunctionPointer)(const Event * Ev);

struct Functor
{
	Object * Obj = nullptr;
	MemberFunctionPointer Fptr = nullptr;
	
	Functor() = default;

	Functor(Object * Obj, const MemberFunctionPointer Func) : Obj(Obj), Fptr(Func)
	{
	}

	INLINE void operator()(const Event * Ev) const
	{
		(*Fptr)(Ev);
	}
};