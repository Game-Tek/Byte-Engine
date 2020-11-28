#pragma once

#include <GTSL/Vector.hpp>

#include "ByteEngine/Application/AllocatorReferences.h"

#include "ByteEngine/Resources/FontResourceManager.h"
#include <GTSL/Math/Vector2.h>

#include "ByteEngine/Debug/Assert.h"

struct LinearBezier
{
	LinearBezier(GTSL::Vector2 a, GTSL::Vector2 b) : Points{ a, b } {}
	GTSL::Vector2 Points[2];

	bool IsPerfectlyVertical() const
	{
		return Points[0].X == Points[1].X;
	}

	bool IsPerfectlyHorizontal() const
	{
		return Points[0].Y == Points[1].Y;
	}
};

struct CubicBezier
{
	CubicBezier(GTSL::Vector2 a, GTSL::Vector2 b, GTSL::Vector2 c) : Points{ a, b, c } {}
	GTSL::Vector2 Points[3];

	bool IsPerfectlyVertical() const
	{
		return Points[0].X == Points[1].X == Points[2].X;
	}

	bool IsPerfectlyHorizontal() const
	{
		return Points[0].Y == Points[1].Y == Points[2].Y;
	}
};

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

	FaceTree(const BE::PersistentAllocatorReference allocator) : Faces(64, allocator)
	{
		
	}

	void MakeFromPaths(const FontResourceManager::Font& font, const BE::PAR& allocator)
	{
		for(auto& e : font.Glyphs)
		{
			auto& glyph = e.second;

			Faces.EmplaceBack();
			auto& face = Faces.back();
			face.LinearBeziers.Initialize(16, allocator);
			face.CubicBeziers.Initialize(16, allocator);
			
			for(const auto& path : glyph.Paths)
			{
				for(const auto& segment : path.Segments)
				{
					if(segment.IsBezierCurve())
					{
						face.CubicBeziers.EmplaceBack(segment.Points[0], segment.Points[1], segment.Points[2]);
					}
					else
					{
						face.LinearBeziers.EmplaceBack(segment.Points[0], segment.Points[2]);
					}
				}
			}
		}
	}

	struct Band
	{
		GTSL::Vector<uint32, BE::PAR> Lines;
		GTSL::Vector<uint32, BE::PAR> Curves;
	};
	
	struct Face
	{
		GTSL::Vector<LinearBezier, BE::PersistentAllocatorReference> LinearBeziers;
		GTSL::Vector<CubicBezier, BE::PersistentAllocatorReference> CubicBeziers;

		GTSL::Vector<Band, BE::PAR> HorizontalBars;
		GTSL::Vector<Band, BE::PAR> VerticalBars;
	};

	GTSL::Vector<Face, BE::PAR> Faces;
};