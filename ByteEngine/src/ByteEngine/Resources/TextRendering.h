#pragma once

#include <GTSL/Vector.hpp>
#include <GTSL/Math/Math.hpp>


#include "ByteEngine/Application/AllocatorReferences.h"

#include "ByteEngine/Resources/FontResourceManager.h"
#include <GTSL/Math/Vector2.h>

#include "ByteEngine/Debug/Assert.h"

#define STB_IMAGE_WRITE_IMPLEMENTATION
#define STBI_MSC_SECURE_CRT
#include "stb image/stb_image_write.h"

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

struct FaceTree : public Object
{	
	FaceTree(const BE::PersistentAllocatorReference allocator) : Faces(64, allocator)
	{
		
	}

	//Lower index bands represent lower Y locations
	//Fonts are in the range 0 <-> 1
	void MakeFromPaths(const FontResourceManager::Font& font, const BE::PAR& allocator)
	{
		const auto& glyph = font.Glyphs.at(font.GlyphMap.at('M'));

		Faces.EmplaceBack();
		auto& face = Faces.back();
		face.LinearBeziers.Initialize(16, allocator);
		face.CubicBeziers.Initialize(16, allocator);

		auto minBBox = GTSL::Vector2(glyph.BoundingBox[0], glyph.BoundingBox[1]);
		auto maxBBox = GTSL::Vector2(glyph.BoundingBox[2], glyph.BoundingBox[3]);

		//BE_LOG_MESSAGE("Min BBOx: ", minBBox.X, " ", minBBox.Y)
		//BE_LOG_MESSAGE("Max BBOx: ", maxBBox.X, " ", maxBBox.Y)
		
		for(const auto& path : glyph.Paths)
		{
			for(const auto& segment : path.Segments)
			{
				if(segment.IsBezierCurve())
				{
					GTSL::Vector2 postPoints[3];
					
					//BE_LOG_MESSAGE("Pre Curve")
					//
					//BE_LOG_MESSAGE("P0: ", segment.Points[0].X, " ", segment.Points[0].Y)
					//BE_LOG_MESSAGE("CP: ", segment.Points[1].X, " ", segment.Points[1].Y)
					//BE_LOG_MESSAGE("P1: ", segment.Points[2].X, " ", segment.Points[2].Y)

					postPoints[0] = GTSL::Math::MapToRange(segment.Points[0], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));
					postPoints[1] = GTSL::Math::MapToRange(segment.Points[1], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));
					postPoints[2] = GTSL::Math::MapToRange(segment.Points[2], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));

					//BE_LOG_MESSAGE("Post Curve")
					//
					//BE_LOG_MESSAGE("P0: ", postPoints[0].X, " ", postPoints[0].Y)
					//BE_LOG_MESSAGE("CP: ", postPoints[1].X, " ", postPoints[1].Y)
					//BE_LOG_MESSAGE("P1: ", postPoints[2].X, " ", postPoints[2].Y)
					
					face.CubicBeziers.EmplaceBack(postPoints[0], postPoints[1], postPoints[2]);
				}
				else
				{
					GTSL::Vector2 postPoints[2];
					
					//BE_LOG_MESSAGE("Pre Line")
					//
					//BE_LOG_MESSAGE("P0: ", segment.Points[0].X, " ", segment.Points[0].Y)
					//BE_LOG_MESSAGE("P1: ", segment.Points[2].X, " ", segment.Points[2].Y)

					postPoints[0] = GTSL::Math::MapToRange(segment.Points[0], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));
					postPoints[1] = GTSL::Math::MapToRange(segment.Points[2], minBBox, maxBBox, GTSL::Vector2(0.0f, 0.0f), GTSL::Vector2(1.0f, 1.0f));

					//BE_LOG_MESSAGE("Post Line")
					//
					//BE_LOG_MESSAGE("P0: ", postPoints[0].X, " ", postPoints[0].Y)
					//BE_LOG_MESSAGE("P1: ", postPoints[1].X, " ", postPoints[1].Y)
					
					face.LinearBeziers.EmplaceBack(postPoints[0], postPoints[1]);
				}

				//BE_LOG_MESSAGE("");
			}
		}

		face.Bands.Initialize(BANDS, allocator);
		for(uint16 i = 0; i < BANDS; ++i)
		{
			auto& e = face.Bands[face.Bands.EmplaceBack()];
			e.Lines.Initialize(8, allocator); e.Curves.Initialize(8, allocator);
		}

		auto GetBandsForLinear = [&](const LinearBezier& linearBezier, uint16& from, uint16& to) -> void
		{
			auto height = 1.0f / (float32)(BANDS);
			auto min = 0.0f; auto max = height;

			for(uint16 i = 0; i < BANDS; ++i)
			{
				if(linearBezier.Points[0].Y >= min && linearBezier.Points[0].Y <= max)
				{
					from = i;
				}

				if(linearBezier.Points[1].Y >= min && linearBezier.Points[1].Y <= max)
				{
					to = i;
				}
				
				min += height;
				max += height;
			}
			
			//from = GTSL::Math::Clamp(uint16(linearBezier.Points[0].Y * static_cast<float32>(BANDS)), uint16(0), uint16(BANDS - 1));
			//to   = GTSL::Math::Clamp(uint16(linearBezier.Points[1].Y * static_cast<float32>(BANDS)), uint16(0), uint16(BANDS - 1));
		};
		
		auto GetBandsForCubic = [&](const CubicBezier& cubicBezier, uint16& from, uint16& to) -> void
		{
			auto height = 1.0f / (float32)(BANDS);
			auto min = 0.0f; auto max = height;

			for(uint16 i = 0; i < BANDS; ++i)
			{
				if(cubicBezier.Points[0].Y >= min && cubicBezier.Points[0].Y <= max)
				{
					from = i;
				}

				if(cubicBezier.Points[2].Y >= min && cubicBezier.Points[2].Y <= max)
				{
					to = i;
				}
				
				min += height;
				max += height;
			}
			
			//from = GTSL::Math::Clamp(uint16(cubicBezier.Points[0].Y * static_cast<float32>(BANDS)), uint16(0), uint16(BANDS - 1));
			//to   = GTSL::Math::Clamp(uint16(cubicBezier.Points[2].Y * static_cast<float32>(BANDS)), uint16(0), uint16(BANDS - 1));
		};
		
		for(uint16 l = 0; l < face.LinearBeziers.GetLength(); ++l)
		{
			uint16 from, to;
			GetBandsForLinear(face.LinearBeziers[l], from, to);
			for(uint16 b = from; b < to + 1; ++b) { face.Bands[b].Lines.EmplaceBack(l); }
		}
		
		for(uint16 c = 0; c < face.CubicBeziers.GetLength(); ++c)
		{
			uint16 from, to;
			GetBandsForCubic(face.CubicBeziers[c], from, to);
			for(uint16 b = from; b < to + 1; ++b) { face.Bands[b].Curves.EmplaceBack(c); }
		}
	}

	float32 Eval(GTSL::Vector2 point, GTSL::Vector2 iResolution, uint16 ch)
	{
		auto testSide = [](const GTSL::Vector2 a, const GTSL::Vector2 b, const GTSL::Vector2 p)
		{
			return ((a.X - b.X) * (p.Y - b.Y) - (a.Y - b.Y) * (p.X - b.X));
		};

		auto getBandIndex = [](const GTSL::Vector2 pos)
		{
			return GTSL::Math::Clamp(static_cast<uint16>(pos.Y * static_cast<float32>(BANDS)), static_cast<uint16>(0), uint16(BANDS - 1));
		};
		
		auto& face = Faces[0];

		auto& band = face.Bands[getBandIndex(point)];

		float32 result = 0.0f; float32 lowestLength = 0.0f;
		
		{
			{
				uint16 closestLineSegment = 0;
				float32 minLength = 1000.0f;
				
				for(uint8 i = 0; i < band.Lines.GetLength(); ++i)
				{
					auto line = face.LinearBeziers[band.Lines[i]];

					GTSL::Vector2 min, max;
					
					if (line.Points[0].X <= line.Points[1].X)
					{ min.X = line.Points[0].X; max.X = line.Points[1].X; }
					else { min.X = line.Points[1].X; max.X = line.Points[0].X; }
					
					if (line.Points[0].Y <= line.Points[1].Y)
					{ min.Y = line.Points[0].Y; max.Y = line.Points[1].Y; }
					else { min.Y = line.Points[1].Y; max.Y = line.Points[0].Y; }

					if(point.X >= min.X && point.X <= max.X || point.Y >= min.Y && point.Y <= max.Y)
					{
						auto pointLine = GTSL::Math::ClosestPointOnLineSegmentToPoint(line.Points[0], line.Points[1], point);
						auto dist = GTSL::Math::LengthSquared(point, pointLine);
						
						if(dist <= minLength) { minLength = dist; closestLineSegment = band.Lines[i]; }
					}
				}

				auto line = face.LinearBeziers[closestLineSegment];
				auto pixelsThree = 0.01f / iResolution.X;
				auto side = testSide(line.Points[0], line.Points[1], point) > 0.0f ? 1.0f : -1.0f;
				
				result = GTSL::Math::MapToRange(GTSL::Math::Clamp(minLength * side, 0.0f, pixelsThree), 0.0f, pixelsThree, 0.0f, 1.0f);

				lowestLength = minLength;
			}
			
			//BE_LOG_MESSAGE("X: ", point.X, " Y: ", point.Y, " C. Segment[0]: X ", line0.Points[0].X, " Y ", line0.Points[0].Y, " C. Segment[1]: X ", line0.Points[1].X, " Y ", line0.Points[1].Y, " Best Distance: ", minLength)
			//return testSide(line.Points[0], line.Points[1], point) >= 0.0f ? 1.0f : 0.0f;

			{
				float32 minLength = 100.0f; GTSL::Vector2 closestAB, closestBC;
				
				auto evalBezier = [](const CubicBezier segment, float32 t)
				{
					auto ab = GTSL::Math::Lerp(segment.Points[0], segment.Points[1], t);
					auto bc = GTSL::Math::Lerp(segment.Points[1], segment.Points[2], t);
					return GTSL::Math::Lerp(ab, bc, t);
				};
				
				for(uint8 i = 0; i < band.Curves.GetLength(); ++i)
				{
					const auto& curve = face.CubicBeziers[band.Curves[i]];
				
					GTSL::Vector2 min, max;
					
					if (curve.Points[0].X <= curve.Points[2].X) {
						min.X = curve.Points[0].X; max.X = curve.Points[2].X;
					}
					else {
						min.X = curve.Points[2].X; max.X = curve.Points[0].X;
					}
				
					if (curve.Points[0].Y <= curve.Points[2].Y) {
						min.Y = curve.Points[0].Y; max.Y = curve.Points[2].Y;
					}
					else {
						min.Y = curve.Points[2].Y; max.Y = curve.Points[0].Y;
					}
				
					if(point.X >= min.X && point.X <= max.X || point.Y >= min.Y && point.Y <= max.Y)
					{
						////GTSL::Vector2 percent = ((point / iResolution) - GTSL::Vector2(0.25,0.5));
						//GTSL::Vector2 percent(0, 0);
						//percent.X *= (iResolution.X / iResolution.Y); //TODO: CHECK HOW RESOLUTION COMES INTO PLAY      
						//GTSL::Vector2 v0 = curve.Points[2] - curve.Points[0];
						//GTSL::Vector2 v1 = curve.Points[1] - curve.Points[0];
						//GTSL::Vector2 v2 = percent - curve.Points[0];
						//float32 dot00 = GTSL::Math::DotProduct(v0, v0); float32 dot01 = GTSL::Math::DotProduct(v0, v1);
						//float32 dot02 = GTSL::Math::DotProduct(v0, v2); float32 dot11 = GTSL::Math::DotProduct(v1, v1);
						//float32 dot12 = GTSL::Math::DotProduct(v1, v2);
						//const float32 invDenom = 1.0f / (dot00 * dot11 - dot01 * dot01);
						//float32 u = (dot11 * dot02 - dot01 * dot12) * invDenom;
						//float32 v = (dot00 * dot12 - dot01 * dot02) * invDenom;
						//// use the blinn and loop method					
						//counter += GTSL::Math::Sign(1.0f - u - v);

						float32 dist = 100.0f;  GTSL::Vector2 csAB, csBC;

						constexpr uint16 LOOPS = 128;
						
						for(uint32 i = 0; i < LOOPS; ++i)
						{
							auto t = static_cast<float32>(i) / float32(LOOPS - 1);
							auto ab = GTSL::Math::Lerp(curve.Points[0], curve.Points[1], t);
							auto bc = GTSL::Math::Lerp(curve.Points[1], curve.Points[2], t);
							auto pos = GTSL::Math::Lerp(ab, bc, t);
							auto newDist = GTSL::Math::LengthSquared(point, pos);

							if(newDist < dist) { dist = newDist; csAB = ab; csBC = bc; }
						}

						if(dist < minLength) { minLength = dist; closestAB = csAB; closestBC = csBC; }
					}
				}

				if(minLength < lowestLength)
				{
					auto pixelsThree = 0.01f / iResolution.X;
					auto side = testSide(closestAB, closestBC, point) > 0.0f ? 1.0f : -1.0f;
				
					result = GTSL::Math::MapToRange(GTSL::Math::Clamp(minLength * side, 0.0f, pixelsThree), 0.0f, pixelsThree, 0.0f, 1.0f);
					
					lowestLength = minLength;
				}
			}
		}

		return result;
	}

	static constexpr uint16 BANDS = 1;
	
	void RenderChar(GTSL::Extent2D res, uint16 ch, const BE::PAR& allocator)
	{
		GTSL::Buffer buffer; buffer.Allocate(res.Width * res.Width, 8, allocator);
		
		for(uint16 xr = 0, x = 0; xr < res.Width; ++xr, ++x)
		{
			for(uint16 yr = 0, y = res.Height - 1; yr < res.Height; ++yr, --y)
			{
				buffer.GetData()[xr + yr * res.Height] = Eval(GTSL::Vector2(x / static_cast<float32>(res.Width), y / static_cast<float32>(res.Height)), GTSL::Vector2(res.Width, res.Height), ch) * 255;
			}
		}

		stbi_write_bmp("A_CharRender.bmp", res.Width, res.Height, 1, buffer.GetData());
		
		buffer.Free(8, allocator);
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
	
	GTSL::Vector<Face, BE::PAR> Faces;
};