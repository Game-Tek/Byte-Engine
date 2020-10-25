#pragma once

#include <GTSL/Vector.hpp>

#include "ByteEngine/Application/AllocatorReferences.h"

#include "ByteEngine/Resources/FontResourceManager.h"
#include <GTSL\Bitman.h>
#include <GTSL/Math/Vector2.h>
#include <GTSL/Math/Line.h>

#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Utility/Shapes/Box.h"

struct FontPoint
{
	FontPoint() = default;
	FontPoint(int32 x, int32 y) : X(x), Y(y) {}

	int32 X, Y;
};

struct LinearBezier
{
	LinearBezier(FontPoint a, FontPoint b) : Points{ a, b } {}
	FontPoint Points[2];

	[[nodiscard]] bool IsPerfectlyVertical() const { return Points[0].X == Points[1].X; }
	[[nodiscard]] bool IsPerfectlyHorizontal() const { return Points[0].Y == Points[1].Y; }
};

struct CubicBezier
{
	CubicBezier(FontPoint a, FontPoint b, FontPoint c) : Points{ a, b, c } {}
	FontPoint Points[3];
};

FontPoint makePoint(const GTSL::Vector2 vector) { return FontPoint(vector.X, vector.Y); }

struct Quad
{	
	GTSL::Vector<uint16, BE::PAR> curves;
	GTSL::Vector<uint16, BE::PAR> lines;
};

struct Line
{
	GTSL::Vector2 Start, End;
};

inline bool LinevLine(const Line l1, const Line l2, GTSL::Vector2* i)
{
	GTSL::Vector2 s1, s2;
	s1.X = l1.End.X - l1.Start.X; s1.Y = l1.End.Y - l1.Start.Y;
	s2.X = l2.End.X - l2.Start.X; s2.Y = l2.End.Y - l2.Start.Y;

	const float32 div = -s2.X * s1.Y + s1.X * s2.Y;
	if (div == 0.0f) { BE_ASSERT(false, "") }
	const float s = (-s1.Y * (l1.Start.X - l2.Start.X) + s1.X * (l1.Start.Y - l2.Start.Y)) / div;
	const float t = (s2.X * (l1.Start.Y - l2.Start.Y) - s2.Y * (l1.Start.X - l2.Start.X)) / div;

	if (s >= 0 && s <= 1 && t >= 0 && t <= 1)
	{
		// Collision detected
		if (i)
		{
			i->X = l1.Start.X + (t * s1.X);
			i->Y = l1.Start.Y + (t * s1.Y);
		}
		
		return true;
	}

	return false; // No collision
}

//inline bool Box_V_Line(const Box box, const LinearBezier linearBezier)
//{
//	auto a = LinevLine(box.GetTopLine(), linearBezier, nullptr);
//	auto b = LinevLine(box.GetRightLine(), linearBezier, nullptr);
//	auto c = LinevLine(box.GetBottomLine(), linearBezier, nullptr);
//	auto d = LinevLine(box.GetLeftLine(), linearBezier, nullptr);
//
//	return a || b || c || d;
//}

struct FaceTree
{
	FaceTree() = default;

	FaceTree(const BE::PersistentAllocatorReference allocator) : cubicBeziers(64, allocator), linearBeziers(64, allocator)
	{}

	void MakeFromPaths(const FontResourceManager::Glyph& outline, const BE::TAR& allocator)
	{
		auto& curves = outline.Paths[0].Segments;
		
		for(uint16 i = 0; i < curves.GetLength(); ++i)
		{
			if(curves[i].IsBezierCurve())
			{
				cubicBeziers.EmplaceBack(makePoint(curves[i].Points[0]), makePoint(curves[i].Points[1]), makePoint(curves[i].Points[2]));
			}
			else
			{
				linearBeziers.EmplaceBack(makePoint(curves[i].Points[0]), makePoint(curves[i].Points[2]));
			}
		}

		//for(uint32 level = 0; level < 10/*max levels*/; ++level)
		//{
		//	ContentType content;
		//	Quad quad;
		//
		//	if(quad.IntersectionCount())
		//	{
		//		//subdivide
		//	}
		//	else
		//	{
		//		blankOrFilledQuads.EmplaceBack(0/*quad index*/);
		//	}
		//	
		//	//quads[level][]
		//}
		
		//intersect with grid
	}

	
	GTSL::Vector<Quad, BE::PersistentAllocatorReference> quads[10];

	GTSL::Vector<CubicBezier, BE::PersistentAllocatorReference> cubicBeziers;
	GTSL::Vector<LinearBezier, BE::PersistentAllocatorReference> linearBeziers;

	Quad firstQuad;
};