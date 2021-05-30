#pragma once

#include <GTSL/Vector.hpp>
#include <GTSL/Math/Math.hpp>
#include "ByteEngine/Application/AllocatorReferences.h"
#include "ByteEngine/Resources/FontResourceManager.h"
#include <GTSL/Math/Vectors.h>
#include "ByteEngine/Debug/Assert.h"

#define STB_IMAGE_WRITE_IMPLEMENTATION
#define STBI_MSC_SECURE_CRT
#include "stb image/stb_image_write.h"

#undef MinMax

struct LinearBezier
{
	LinearBezier(GTSL::Vector2 a, GTSL::Vector2 b) : Points{ a, b } {}
	GTSL::Vector2 Points[2];
};

struct CubicBezier
{
	CubicBezier(GTSL::Vector2 a, GTSL::Vector2 b, GTSL::Vector2 c) : Points{ a, b, c } {}
	GTSL::Vector2 Points[3];
};

inline float det(GTSL::Vector2 a, GTSL::Vector2 b) { return a.X() * b.Y() - b.X() * a.Y(); }
// Find vector vi given pixel p=(0,0) and B�zier points b0, b1, b2
inline GTSL::Vector2 get_distance_vector(GTSL::Vector2 b0, GTSL::Vector2 b1, GTSL::Vector2 b2) {
	float a = det(b0, b2), b = 2 * det(b1, b0), d = 2 * det(b2, b1); // ab,c(p)
	float f = b * d - a * a; // f(p)
	
	GTSL::Vector2 d21 = b2 - b1, d10 = b1 - b0, d20 = b2 - b0;
	
	GTSL::Vector2 gf = (d21 * b + d10 * d + d20 * a) * 2.0f;
	gf = GTSL::Vector2(gf.Y(), -gf.X()); // delta f(p)
	GTSL::Vector2 pp = gf * -f / GTSL::Math::DotProduct(gf, gf); // p'
	GTSL::Vector2 d0p = b0 - pp; // p' to origin
	float ap = det(d0p, d20), bp = 2 * det(d10, d0p); // a,b(p')
	// (note that 2*ap+bp+dp=2*a+b+d=4*area(b0,b1,b2))
	float t = GTSL::Math::Clamp((ap + bp) / (2 * a + b + d), 0.0f, 1.0f); // t-
	return GTSL::Math::Lerp(GTSL::Math::Lerp(b0, b1, t), GTSL::Math::Lerp(b1, b2, t), t); // vi = bc(t-)
}

struct Band
{
	GTSL::Vector<uint16, BE::PAR> Lines;
	GTSL::Vector<uint16, BE::PAR> Curves;
};

struct Face
{
	GTSL::Vector<LinearBezier, BE::PersistentAllocatorReference> LinearBeziers;
	GTSL::Vector<CubicBezier, BE::PersistentAllocatorReference> CubicBeziers;

	GTSL::Vector<Band, BE::PAR> Bands;
};

//Lower index bands represent lower Y locations
//Fonts are in the range 0 <-> 1
inline void MakeFromPaths(const FontResourceManager::Glyph& glyph, Face& face, const uint16 bands, const BE::PAR& allocator)
{
	face.LinearBeziers.Initialize(16, allocator);
	face.CubicBeziers.Initialize(16, allocator);

	auto minBBox = glyph.BoundingBox[0]; auto maxBBox = glyph.BoundingBox[1];

	for (const auto& path : glyph.Paths) {
		for (const auto& segment : path) {
			if (segment.IsBezierCurve()) {
				GTSL::Vector2 postPoints[3];

				postPoints[0] = GTSL::Math::MapToRange(segment.Points[0], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));
				postPoints[1] = GTSL::Math::MapToRange(segment.Points[1], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));
				postPoints[2] = GTSL::Math::MapToRange(segment.Points[2], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));

				face.CubicBeziers.EmplaceBack(postPoints[0], postPoints[1], postPoints[2]);
			}
			else {
				GTSL::Vector2 postPoints[2];

				postPoints[0] = GTSL::Math::MapToRange(segment.Points[0], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));
				postPoints[1] = GTSL::Math::MapToRange(segment.Points[2], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));

				face.LinearBeziers.EmplaceBack(postPoints[0], postPoints[1]);
			}
		}
	}

	face.Bands.Initialize(bands, allocator);

	for (uint16 i = 0; i < bands; ++i) {
		auto& e = face.Bands.EmplaceBack();
		e.Lines.Initialize(8, allocator); e.Curves.Initialize(8, allocator);
	}

	auto GetBandsForLinear = [&](const LinearBezier& linearBezier, uint16& from, uint16& to) -> void {
		auto height = 1.0f / static_cast<float32>(bands);
		auto min = 0.0f; auto max = height;

		from = GTSL::Math::Clamp(uint16(linearBezier.Points[0].Y() * static_cast<float32>(bands)), uint16(0), uint16(bands - 1));
		to = GTSL::Math::Clamp(uint16(linearBezier.Points[1].Y() * static_cast<float32>(bands)), uint16(0), uint16(bands - 1));
	};

	auto GetBandsForCubic = [&](const CubicBezier& cubicBezier, uint16& from, uint16& to) -> void {
		auto height = 1.0f / static_cast<float32>(bands);
		auto min = 0.0f; auto max = height;

		from = GTSL::Math::Clamp(uint16(cubicBezier.Points[0].Y() * static_cast<float32>(bands)), uint16(0), uint16(bands - 1));
		to = GTSL::Math::Clamp(uint16(cubicBezier.Points[2].Y() * static_cast<float32>(bands)), uint16(0), uint16(bands - 1));
	};

	for (uint16 l = 0; l < face.LinearBeziers.GetLength(); ++l) {
		uint16 from, to;
		GetBandsForLinear(face.LinearBeziers[l], from, to); GTSL::Math::MinMax(from, to, from, to);
		for (uint16 b = from; b < to + 1; ++b) { face.Bands[b].Lines.EmplaceBack(l); }
	}

	for (uint16 c = 0; c < face.CubicBeziers.GetLength(); ++c) {
		uint16 from, to;
		GetBandsForCubic(face.CubicBeziers[c], from, to); GTSL::Math::MinMax(from, to, from, to);
		for (uint16 b = from; b < to + 1; ++b) { face.Bands[b].Curves.EmplaceBack(c); }
	}
}

float32 Eval(GTSL::Vector2 point, GTSL::Vector2 iResolution, uint16 ch)
{
	constexpr auto AA_LENGTH = 0.001f; constexpr uint16 BANDS = 4;

	auto getBandIndex = [](const GTSL::Vector2 pos) {
		return GTSL::Math::Clamp(static_cast<uint16>(pos.Y() * static_cast<float32>(BANDS)), static_cast<uint16>(0), uint16(BANDS - 1));
	};
	
	auto face = Face();

	auto& band = face.Bands[getBandIndex(point)];

	float32 result = 0.0f; float32 lowestLength = 100.0f;
	
			
	for(uint8 i = 0; i < band.Lines.GetLength(); ++i)
	{
		auto line = face.LinearBeziers[band.Lines[i]];

		GTSL::Vector2 min, max;
		
		GTSL::Math::MinMax(line.Points[0], line.Points[1], min, max);

		if(GTSL::Math::PointInBoxProjection(min, max, point)) {
			float32 isOnSegment;
			auto pointLine = GTSL::Math::ClosestPointOnLineSegmentToPoint(line.Points[0], line.Points[1], point, isOnSegment);
			auto dist = GTSL::Math::LengthSquared(point, pointLine);
			
			if(dist < lowestLength) {
				lowestLength = dist;
				auto side = GTSL::Math::TestPointToLineSide(line.Points[0], line.Points[1], point) > 0.0f ? 1.0f : 0.0f;
				result = GTSL::Math::MapToRange(GTSL::Math::Clamp(lowestLength, 0.0f, AA_LENGTH), 0.0f, AA_LENGTH, 0.0f, 1.0f) * side;
				//result = GTSL::Math::TestPointToLineSide(line.Points[0], line.Points[1], point) >= 0.0f ? 1.0f : 0.0f;
			}
		}
	}

	{
		GTSL::Vector2 closestAB, closestBC;
		
		for(uint8 i = 0; i < band.Curves.GetLength(); ++i)
		{
			const auto& curve = face.CubicBeziers[band.Curves[i]];
		
			GTSL::Vector2 min, max;
			
			GTSL::Math::MinMax(curve.Points[0], curve.Points[2], min, max);
		
			if(GTSL::Math::PointInBoxProjection(min, max, point))
			{
				float32 dist = 100.0f;

				constexpr uint16 LOOPS = 32; float32 bounds[2] = { 0.0f, 1.0f };

				uint8 sideToAdjust = 0;

				for (uint32 l = 0; l < LOOPS; ++l)
				{
					for (uint8 i = 0, ni = 1; i < 2; ++i, --ni)
					{
						auto t = GTSL::Math::Lerp(bounds[0], bounds[1], static_cast<float32>(i) / 1.0f);
						auto ab = GTSL::Math::Lerp(curve.Points[0], curve.Points[1], t);
						auto bc = GTSL::Math::Lerp(curve.Points[1], curve.Points[2], t);
						auto pos = GTSL::Math::Lerp(ab, bc, t);
						auto newDist = GTSL::Math::LengthSquared(pos, point);

						if (newDist < dist) { sideToAdjust = ni; dist = newDist; closestAB = ab; closestBC = bc; }
					}

					bounds[sideToAdjust] = (bounds[0] + bounds[1]) / 2.0f;
				}

				if (dist < lowestLength)
				{
					lowestLength = dist;

					auto side = GTSL::Math::TestPointToLineSide(closestAB, closestBC, point) > 0.0f ? 1.0f : 0.0f;
					result = GTSL::Math::MapToRange(GTSL::Math::Clamp(lowestLength, 0.0f, AA_LENGTH), 0.0f, AA_LENGTH, 0.0f, 1.0f) * side;

					//result = GTSL::Math::TestPointToLineSide(closestAB, closestBC, point) >= 0.0f ? 1.0f : 0.0f;
				}
			}
		}
	}

	return result;
}

inline void RenderChar(GTSL::Extent2D res, uint16 ch, const BE::PAR& allocator)
{
	GTSL::Buffer<BE::PAR> buffer; buffer.Allocate(res.Width * res.Width, 8, allocator);
	
	for(uint16 xr = 0, x = 0; xr < res.Width; ++xr, ++x)
	{
		for(uint16 yr = 0, y = res.Height - 1; yr < res.Height; ++yr, --y)
		{
			buffer.GetData()[xr + yr * res.Height] = Eval(GTSL::Vector2(x / static_cast<float32>(res.Width), y / static_cast<float32>(res.Height)), GTSL::Vector2(res.Width, res.Height), ch) * 255;
		}
	}

	stbi_write_bmp("A_CharRender.bmp", res.Width, res.Height, 1, buffer.GetData());
}