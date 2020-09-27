#pragma once

#include <GTSL/Vector.hpp>

#include "ByteEngine/Application/AllocatorReferences.h"

#include <freetype/freetype.h>
#include <GTSL\Bitman.h>
#include <GTSL/Math/Vector2.h>
#include <GTSL/Math/Line.h>

#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Utility/Shapes/Box.h"

enum class ContentType
{
	BLANK,
	FILL,
	HOR_LINE,
	VER_LINE,
	LINEAR_BEZIER,
	CUADRATIC_BEZIER,
	HOR_AND_VER,
	HOR_AND_LINEAR,
	VER_AND_LINEAR,
	LINEAR_AND_LINEAR,
	HOR_AND_CUADRTIC,
	VER_AND_CUADRATIC,
	LINEAR_AND_CUADRATIC,
	CUADRATIC_AND_CUADRATIC
};

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

	[[nodiscard]] float32 EvaluateAsFunction(const float32 x) const
	{
		BE_ASSERT((Points[1].X - Points[0].X) != 0)

			auto tanX = ((Points[1].Y - Points[0].Y) / (Points[1].X - Points[0].X)) * x;

		auto b = Points[0].Y + (-tanX);

		return tanX + b;
	}

	[[nodiscard]] float32 EvaluateAsRoot(float32 y) const
	{
		BE_ASSERT((Points[1].X - Points[0].X) != 0)

			//auto tanX = ((Points[1].Y - Points[0].Y) / (Points[1].X - Points[0].X)) * 0;

			auto x = 0.0f;

		auto b = Points[0].Y;

		x = y -= b;

		y /= ((Points[1].Y - Points[0].Y) / (Points[1].X - Points[0].X));

		return x;
	}

	[[nodiscard]] bool IsPerfectlyVertical() const { return Points[0].X == Points[1].X; }
	[[nodiscard]] bool IsPerfectlyHorizontal() const { return Points[0].Y == Points[1].Y; }
};

struct CubicBezier
{
	CubicBezier(FontPoint a, FontPoint b, FontPoint c) : Points{ a, b, c } {}
	FontPoint Points[3];
};

struct HorizontalLine
{
	//if line pointing right fill right side (of shape) or bottom of global
	HorizontalLine(const LinearBezier line) : FillBottomSide(line.Points[0].X < line.Points[1].X) {}
	
	int16 Y;
	uint16 FillBottomSide;
};

struct VerticalLine
{
	//if line pointing downward fill right side (of shape) or left of global
	VerticalLine(const LinearBezier line) : FillLeftSide(line.Points[0].Y > line.Points[1].Y) {}
	
	int16 X;
	uint16 FillLeftSide;
};

struct Line
{
	Line(const LinearBezier line) : L(line), PaintRightIf(line.Points[0].X < line.Points[1].X && line.Points[0].Y > line.Points[1].Y)
	{
	}
	
	LinearBezier L;
	uint16 PaintRightIf;
};

struct Cubic
{
	Cubic(const CubicBezier line) : L(line), PaintRightIf(line.Points[0].X < line.Points[1].X)
	{
	}
	
	CubicBezier L;
	uint16 PaintRightIf;
};

bool isControlPoint(uint32 flags) { return !GTSL::CheckBit(0, flags); }

FontPoint makePoint(const FT_Vector vector) { return FontPoint(vector.x, vector.y); }

void t()
{
	FT_Outline outline;
}

struct Quad
{
	uint32 IntersectionCount() const { return 5; }
private:
};

bool intersectBoxLine(const Box box, const LinearBezier linearBezier)
{
	float32 boxRightWallX = 0.0f;
	float32 boxLeftWallX = 0.0f;
	float32 boxBottomWallY = 0.0f;
	float32 boxTopWallY = 0.0f;

	auto lineLeftWallYIntersectionAtX = linearBezier.EvaluateAsFunction(boxLeftWallX);
	auto lineRightWallYIntersectionAtX = linearBezier.EvaluateAsFunction(boxRightWallX);
	
	if(lineLeftWallYIntersectionAtX > boxBottomWallY && lineLeftWallYIntersectionAtX < boxTopWallY)
	{
		if (lineRightWallYIntersectionAtX > boxBottomWallY && lineRightWallYIntersectionAtX < boxTopWallY)
		{
			return true;
		}
	}

	return false;
}

struct FaceTree
{
	FaceTree() = default;

	FaceTree(const BE::PersistentAllocatorReference allocator) : cubicBeziers(64, allocator), linearBeziers(64, allocator), blankOrFilledQuads(32, allocator)
	{}

	void MakeFromPaths(const FT_Outline& outline, const BE::TAR& allocator)
	{		
		for(uint16 i = 0; i < outline.n_contours; ++i)
		{
			FontPoint points[3/*max cuadratic bezier*/];

			uint16 pointInContour = outline.contours[i];
			
			points[0] = makePoint(outline.points[pointInContour]);
			BE_ASSERT(!isControlPoint(outline.tags[pointInContour]));

			++pointInContour;
			
			if(isControlPoint(outline.tags[outline.contours[i] + 1]))
			{
				points[1] = makePoint(outline.points[pointInContour]);
				++pointInContour;
				
				points[2] = makePoint(outline.points[pointInContour]);
				BE_ASSERT(!isControlPoint(outline.tags[pointInContour]));
				//++pointInContour;

				cubicBeziers.EmplaceBack(points[0], points[1], points[2]);
			}
			else
			{
				points[1] = makePoint(outline.points[pointInContour]);
				
				linearBeziers.EmplaceBack(points[0], points[1]);
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

	GTSL::Vector<uint32, BE::PersistentAllocatorReference> blankOrFilledQuads;

	Quad firstQuad;
};