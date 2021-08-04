#pragma once

#include <utility>
#include <array>
#include <GTSL/Pair.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Core.h"
#include "ByteEngine/Application/AllocatorReferences.h"

class Obj
{
public:
	GTSL::Vector3 GetPosition() { return GTSL::Vector3(); }
	
	GTSL::Vector3 GetSupportPointInDirection(const GTSL::Vector3& direction) {
		return GTSL::Vector3();
	}
};

class Simplex
{
public:
	void AddPoint(GTSL::Vector3 newPoint) {
		points[length++] = newPoint;
	}

	void Remove(const uint8 index) {
		for(uint8 i = index; i < length - 1; ++i) {
			points[i] = points[i + 1];
		}
		
		--length;
	}
	
	uint8 GetLength() const { return length; }

	GTSL::Vector3 operator[](const uint8 index) const { return points[3 - index]; }

private:
	GTSL::Vector3 points[4]; uint8 length = 0;
};

struct CollisionInfo
{
	GTSL::Vector3 A, B, Normal;
	float32 Depth;
};

bool GJK() {
	Obj objectA, objectB;
	
	GTSL::Vector3 direction = GTSL::Math::Normalized(objectB.GetPosition() - objectA.GetPosition());
	
	auto supportPoint = objectA.GetSupportPointInDirection(direction) - objectB.GetSupportPointInDirection(-direction);

	Simplex simplex;

	simplex.AddPoint(supportPoint);

	direction = -supportPoint;

	while (true) {
		supportPoint = objectA.GetSupportPointInDirection(direction) - objectB.GetSupportPointInDirection(-direction);

		if(GTSL::Math::DotProduct(supportPoint, direction) <= 0) //if new point doesn't goes past the origin
			return false;

		simplex.AddPoint(supportPoint);

		switch (simplex.GetLength()) {
		case 2: { //line
			auto ab = simplex[1] - simplex[0];
			auto a0 = -simplex[0];

			if (GTSL::Math::DotProduct(ab, a0) > 0.0f) { //
				direction = ((simplex[0] + simplex[1]) * -0.5f);
			} else {
				simplex.Remove(1);
				direction = a0;
			}
			
			break;
		}
		case 3: { //triangle
			auto a = simplex[0], b = simplex[1], c = simplex[2];
			auto ab = b - a;
			auto ac = c - a;
			auto a0 = -a;

			auto abPerp = GTSL::Math::TripleProduct(ac, ab);
			auto acPerp = GTSL::Math::TripleProduct(ab, ac);

			if(GTSL::Math::DotProduct(abPerp, a0) > 0) {
				simplex.Remove(2);
				direction = abPerp;
			} else if (GTSL::Math::DotProduct(acPerp, a0) > 0) {
				simplex.Remove(1);
				direction = acPerp;
			}

			break;
		}
		case 4: { //tetra
			auto a = simplex[0], b = simplex[1], c = simplex[2], d = simplex[3];
			auto ab = b - a, ac = c - a, ad = d - a, a0 = -a;

			auto abc = GTSL::Math::Cross(ab, ac);
			auto acd = GTSL::Math::Cross(ac, ad);
			auto adb = GTSL::Math::Cross(ad, ab);

			if(GTSL::Math::DotProduct(abc, a0) > 0.0f) {
				// the origin is not here, remove d
				simplex.Remove(3);
				direction = abc;
			} else if(GTSL::Math::DotProduct(acd, a0) > 0.0f) {
				simplex.Remove(1);
				direction = acd;
			} else if(GTSL::Math::DotProduct(adb, a0) > 0.0f) {
				simplex.Remove(2);
				direction = adb;
			} else {
				return true;
			}

			break;
		}
		default: __debugbreak();
		}
	}
}

void GetFaceNormals(GTSL::Range<const GTSL::Vector3*> polytope, GTSL::Range<const std::array<uint16, 3>*> indices, auto& normals, uint32& minFace) {
	float32 minDistance = FLT_MAX;

	for(uint32 f = 0; f < indices.ElementCount(); ++f) {
		auto a = polytope[indices[f][0]], b = polytope[indices[f][1]], c = polytope[indices[f][2]];

		auto normal = GTSL::Math::Normalized(GTSL::Math::Cross(b - a, c - a));
		auto distance = GTSL::Math::DotProduct(normal, a);

		if (distance < 0.0f) { //normal check, flip if wrong sided
			normal *= -1.0f; distance *= -1.0f;
		}

		normals.EmplaceBack(normal, distance);

		if(distance < minDistance) {
			minFace = f;
			minDistance = distance;
		}
	}
}

CollisionInfo EPA(const Simplex& simplex) {
	Obj objectA, objectB;

	GTSL::SemiVector<GTSL::Vector3, 64, BE::TAR> polytope{ simplex[0], simplex[1], simplex[2], simplex[3] };
	GTSL::SemiVector<std::array<GTSL::uint16, 3>, 64, BE::TAR> indices{ { 0, 1, 2 }, { 0, 3, 1 }, { 0, 2, 3 }, { 1, 3, 2 } };

	GTSL::SemiVector<GTSL::Pair<GTSL::Vector3, float32>, 64, BE::TAR> normals; uint32 minFace = 0;
	GetFaceNormals(polytope, indices, normals, minFace);

	GTSL::Vector3 minNormal; float32 minDistance = std::numeric_limits<float32>::max();

	while(minDistance == std::numeric_limits<float32>::max()) {
		minNormal = GTSL::Vector3(normals[minFace].First);
		minDistance = normals[minFace].Second;

		auto supportPoint = objectA.GetSupportPointInDirection(minNormal) - objectB.GetSupportPointInDirection(-minNormal);
		float32 sDistance = GTSL::Math::DotProduct(minNormal, supportPoint);

		if(GTSL::Math::Abs(sDistance - minDistance) > 0.001f) {
			minDistance = std::numeric_limits<float32>::max();
			
			GTSL::SemiVector<GTSL::Pair<uint16, uint16>, 64, BE::TAR> uniqueEdges;

			for(uint32 i = 0; i < normals.GetLength(); ++i) {
				if(GTSL::Math::DotProduct(normals[i].First, supportPoint) > 0.0f) {
					auto addIfUniqueEdge = [&](uint32 fIndex, uint32 f0, uint32 f1) {

						if(const auto searchResult = uniqueEdges.Find({ indices[fIndex][f1], indices[fIndex][f0] }); searchResult) {
							uniqueEdges.Pop(searchResult.Get());
						} else {
							uniqueEdges.EmplaceBack(indices[fIndex][f0], indices[fIndex][f1]);
						}
					};

					addIfUniqueEdge(i, 0, 1);
					addIfUniqueEdge(i, 1, 2);
					addIfUniqueEdge(i, 2, 0);

					indices[i][2] = indices.back()[2]; indices[i][1] = indices.back()[1]; indices[i][0] = indices.back()[0];
					indices.PopBack();

					normals[i] = normals.back();
					normals.PopBack();

					--i;
				}
			}

			GTSL::SemiVector<std::array<uint16, 3>, 64, BE::TAR> newFaces;

			for(auto& e : uniqueEdges) {
				newFaces.EmplaceBack(std::array{ e.First, e.Second, static_cast<uint16>(polytope.GetLength()) });
			}

			polytope.EmplaceBack(supportPoint);

			GTSL::SemiVector<GTSL::Pair<GTSL::Vector3, float32>, 64, BE::TAR> newNormals; uint32 newMinFace = 0;
			GetFaceNormals(polytope, indices, newNormals, newMinFace);

			float32 oldMinDistance = std::numeric_limits<float32>::max();
			for(uint32 i = 0; i < normals.GetLength(); ++i) {
				if(normals[i].Second < oldMinDistance) {
					oldMinDistance = normals[i].Second;
					minFace = i;
				}
			}

			if(newNormals[newMinFace].Second < oldMinDistance) {
				minFace = newMinFace + normals.GetLength();
			}

			indices.PushBack(newFaces); normals.PushBack(newNormals);
		}
	}

	CollisionInfo collision_info;
	collision_info.Normal = minNormal;
	collision_info.Depth = minDistance + 0.0001f;

	return collision_info;
}
