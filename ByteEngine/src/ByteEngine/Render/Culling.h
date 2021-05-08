#pragma once

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Vectors.h>

#include "ByteEngine/Core.h"
#include "Math/SIMD/float4.h"

struct AABB;
struct vec4;

//void sse_culling_aabb(AABB* aabb_data, int num_objects, int* culling_res, vec4* frustum_planes)
//{
//	float* aabb_data_ptr = reinterpret_cast(&aabb_data[0]);
//	
//	int* culling_res_sse = &culling_res[0]; //to optimize calculations we gather xyzw elements in separate vectors
//	
//	//float4 zero_v = _mm_setzero_ps();
//	float4 frustum_planes_x[6];
//	float4 frustum_planes_y[6];
//	float4 frustum_planes_z[6];
//	float4 frustum_planes_d[6];
//	
//	int i, j;
//
//	for (i = 0; i < 6; i++)
//	{
//		frustum_planes_x[i] = _mm_set1_ps(frustum_planes.x);
//		frustum_planes_y[i] = _mm_set1_ps(frustum_planes.y);
//		frustum_planes_z[i] = _mm_set1_ps(frustum_planes.z);
//		frustum_planes_d[i] = _mm_set1_ps(frustum_planes.w);
//	}
//
//	float4 zero = _mm_setzero_ps(); //we process 4 objects per step
//
//	for (i = 0; i < num_objects; i += 4)
//	{
//		//load objects data //load aabb min
//		float4 aabb_min_x = _mm_load_ps(aabb_data_ptr);
//		float4 aabb_min_y = _mm_load_ps(aabb_data_ptr + 8);
//		float4 aabb_min_z = _mm_load_ps(aabb_data_ptr + 16);
//		float4 aabb_min_w = _mm_load_ps(aabb_data_ptr + 24);
//		//load aabb max
//		float4 aabb_max_x = _mm_load_ps(aabb_data_ptr + 4);
//		float4 aabb_max_y = _mm_load_ps(aabb_data_ptr + 12);
//		float4 aabb_max_z = _mm_load_ps(aabb_data_ptr + 20);
//		float4 aabb_max_w = _mm_load_ps(aabb_data_ptr + 28);
//
//		aabb_data_ptr += 32;
//
//		//for now we have points in vectors aabb_min_x..w, but for calculations we need to xxxx yyyy zzzz vectors representation - just transpose data
//
//
//
//		_MM_TRANSPOSE4_PS(aabb_min_x, aabb_min_y, aabb_min_z, aabb_min_w);
//		_MM_TRANSPOSE4_PS(aabb_max_x, aabb_max_y, aabb_max_z, aabb_max_w);
//
//	}
//		
//	float4 intersection_res = _mm_setzero_ps();
//	
//	for (j = 0; j < 6; j++) //plane index
//	{
//		//this code is similar to what we make in simple culling
//		//pick closest point to plane and check if it begind the plane. if yes - object outside frustum
//		////dot product, separate for each coordinate, for min & max aabb points
//		float4 aabbMin_frustumPlane_x = _mm_mul_ps(aabb_min_x, frustum_planes_x[j]);
//		float4 aabbMin_frustumPlane_y = _mm_mul_ps(aabb_min_y, frustum_planes_y[j]);
//		float4 aabbMin_frustumPlane_z = _mm_mul_ps(aabb_min_z, frustum_planes_z[j]);
//		float4 aabbMax_frustumPlane_x = _mm_mul_ps(aabb_max_x, frustum_planes_x[j]);
//		float4 aabbMax_frustumPlane_y = _mm_mul_ps(aabb_max_y, frustum_planes_y[j]);
//		float4 aabbMax_frustumPlane_z = _mm_mul_ps(aabb_max_z, frustum_planes_z[j]);
//
//		//we have 8 box points, but we need pick closest point to plane. Just take max
//		float4 res_x = _mm_max_ps(aabbMin_frustumPlane_x, aabbMax_frustumPlane_x);
//		float4 res_y = _mm_max_ps(aabbMin_frustumPlane_y, aabbMax_frustumPlane_y);
//		float4 res_z = _mm_max_ps(aabbMin_frustumPlane_z, aabbMax_frustumPlane_z);
//		
//		//dist to plane = dot(aabb_point.xyz, plane.xyz) + plane.w
//		float4 sum_xy = _mm_add_ps(res_x, res_y);
//		float4 sum_zw = _mm_add_ps(res_z, frustum_planes_d[j]);
//		float4 distance_to_plane = _mm_add_ps(sum_xy, sum_zw);
//		float4 plane_res = _mm_cmple_ps(distance_to_plane, zero);
//	}
//
//	//dist from closest point to plane < 0 ? intersection_res = _mm_or_ps(intersection_res, plane_res); //if yes - aabb behind the plane & outside frustum }
//	//store result
//	//
//	float4i intersection_res_i = _mm_cvtps_epi32(intersection_res);
//	_mm_store_si128((float4i *)&culling_res_sse, intersection_res_i);
//}

float32 projectSphere(const GTSL::Vector3 cameraPosition, const GTSL::Vector3 spherePosition, const float32 radius)
{
	//return GTSL::Math::Tangent(radius / GTSL::Math::Length(cameraPosition, spherePosition));
	return GTSL::Math::Tangent((radius * radius) / GTSL::Math::LengthSquared(spherePosition, cameraPosition));
}