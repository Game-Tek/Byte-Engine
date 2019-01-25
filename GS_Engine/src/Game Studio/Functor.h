#pragma once

#include "Object.h"

#include "Event.h"

typedef void (Object::*MemberFuncPtr)(const Event & Ev);

struct Functor
{
	Object * Obj = nullptr;
	MemberFuncPtr Fptr = nullptr;
	
	Functor()
	{
	}

	Functor(Object * Obj, MemberFuncPtr Func) : Obj(Obj), Fptr(Func)
	{
	}

	INLINE void operator() (const Event & Ev)
	{
		((Obj)->*(Fptr))(Ev);
	}
};