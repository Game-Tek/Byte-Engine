#!/usr/bin/env python3
"""Generates the ACES 2.0 output transform constants that the ACES 2.0 color-grading shader reads.

The ACES 2.0 output transform needs per-hue tables of the display gamut's cusp, the reach gamut's colorfulness, and
the upper hull's gamma. The reference implementation builds them once per display with iterative searches. This
script runs those searches offline for the one display the engine renders to, Rec.709 primaries with a D65 white at
100 nits, and writes the results as BESL constants, so the shader does no table generation at run time.

It is an independent Python port of the Academy's Apache-2.0 reference implementation in
https://github.com/aces-aswf/aces-core (lib/Lib.Academy.OutputTransform.ctl and lib/Lib.Academy.Tonescale.ctl). The
port follows the reference's arithmetic, including its interpolation weights, so its forward transform also serves
as the reference model for the shader's tests.

Usage, from the repository root:
    python3 scripts/generate-aces2-tables.py > crates/byte-engine/assets/rendering/layouts/aces2-rec709-100nit.besl
    python3 scripts/generate-aces2-tables.py --vectors   # prints reference outputs for the shader's tests
"""

import math
import sys

# The display the tables describe: Rec.709 primaries, a D65 white, and a 100-nit peak.
PEAK_LUMINANCE = 100.0
REC709 = ((0.6400, 0.3300), (0.3000, 0.6000), (0.1500, 0.0600), (0.3127, 0.3290))
AP0 = ((0.73470, 0.26530), (0.00000, 1.00000), (0.00010, -0.07700), (0.32168, 0.33767))
AP1 = ((0.713, 0.293), (0.165, 0.830), (0.128, 0.044), (0.32168, 0.33767))
CAM16_PRIMARIES = ((0.8336, 0.1735), (2.3854, -1.4659), (0.087, -0.125), (0.333, 0.333))

TABLE_SIZE = 360
TOTAL_TABLE_SIZE = TABLE_SIZE + 2
BASE_INDEX = 1
HUE_LIMIT = 360.0
CUSP_CORNER_COUNT = 6
MAX_SORTED_CORNERS = 2 * CUSP_CORNER_COUNT
REACH_CUSP_TOLERANCE = 1e-3
DISPLAY_CUSP_TOLERANCE = 1e-7
GAMMA_MINIMUM = 0.0
GAMMA_MAXIMUM = 5.0
GAMMA_SEARCH_STEP = 0.4
GAMMA_ACCURACY = 1e-5

REF_LUMINANCE = 100.0
L_A = 100.0
Y_B = 20.0
SURROUND = (0.9, 0.59, 0.9)
J_SCALE = 100.0
CAM_NL_OFFSET = 0.2713 * 100.0
CAM_NL_SCALE = 4.0 * 100.0
MODEL_GAMMA = SURROUND[1] * (1.48 + math.sqrt(Y_B / REF_LUMINANCE))

CHROMA_COMPRESS = 2.4
CHROMA_COMPRESS_FACT = 3.3
CHROMA_EXPAND = 1.3
CHROMA_EXPAND_FACT = 0.69
CHROMA_EXPAND_THR = 0.5
SMOOTH_CUSPS = 0.12
SMOOTH_M = 0.27
CUSP_MID_BLEND = 1.3
FOCUS_GAIN_BLEND = 0.3
FOCUS_DISTANCE = 1.35
FOCUS_DISTANCE_SCALING = 1.75
COMPRESSION_THRESHOLD = 0.75


# Matrices follow CTL's row-vector convention: a vector times a matrix is v[i] * m[i][j] summed over i.
def mult_f3_f33(v, m):
	return [v[0] * m[0][j] + v[1] * m[1][j] + v[2] * m[2][j] for j in range(3)]


def mult_f33_f33(a, b):
	return [[sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)] for i in range(3)]


def invert_f33(m):
	a, b, c = m[0]
	d, e, f = m[1]
	g, h, i = m[2]
	det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
	return [
		[(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
		[(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
		[(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
	]


def rgb_to_xyz(c, y=1.0):
	(rx, ry), (gx, gy), (bx, by), (wx, wy) = c
	x = wx * y / wy
	z = (1.0 - wx - wy) * y / wy
	d = rx * (by - gy) + bx * (gy - ry) + gx * (ry - by)
	sr = (x * (by - gy) - gx * (y * (by - 1) + by * (x + z)) + bx * (y * (gy - 1) + gy * (x + z))) / d
	sg = (x * (ry - by) + rx * (y * (by - 1) + by * (x + z)) - bx * (y * (ry - 1) + ry * (x + z))) / d
	sb = (x * (gy - ry) - rx * (y * (gy - 1) + gy * (x + z)) + gx * (y * (ry - 1) + ry * (x + z))) / d
	return [
		[sr * rx, sr * ry, sr * (1.0 - rx - ry)],
		[sg * gx, sg * gy, sg * (1.0 - gx - gy)],
		[sb * bx, sb * by, sb * (1.0 - bx - by)],
	]


def xyz_to_rgb(c, y=1.0):
	return invert_f33(rgb_to_xyz(c, y))


AP0_TO_AP1 = mult_f33_f33(rgb_to_xyz(AP0), xyz_to_rgb(AP1))
AP1_TO_AP0 = mult_f33_f33(rgb_to_xyz(AP1), xyz_to_rgb(AP0))


def lerp(a, b, t):
	return a + t * (b - a)


def wrap_to_360(hue):
	y = math.fmod(hue, 360.0)
	return y + 360.0 if y < 0.0 else y


def hue_position_in_uniform_table(hue, table_size):
	return int(wrap_to_360(hue) / HUE_LIMIT * table_size)


def ctl_round(x):
	return int(x - 0.5) if x < 0.0 else int(x + 0.5)


def copysign(x, y):
	return 0.0 if y == 0.0 else math.copysign(abs(x), y)


def compress_fwd_unsigned(rc):
	f = rc**0.42
	return f / (CAM_NL_OFFSET + f)


def compress_inv_unsigned(ra):
	ra = min(ra, 0.99)
	return (CAM_NL_OFFSET * ra / (1.0 - ra)) ** (1.0 / 0.42)


def compress_fwd(v):
	return copysign(compress_fwd_unsigned(abs(v)), v)


def compress_inv(v):
	return copysign(compress_inv_unsigned(abs(v)), v)


class JMhParams:
	"""The CAM's matrices and constants for one RGB space, as the reference's `init_JMhParams` computes them."""

	def __init__(self, primaries):
		matrix_16 = xyz_to_rgb(CAM16_PRIMARIES)
		base = [[2.0, 1.0, 1.0 / 9.0], [1.0, -12.0 / 11.0, 1.0 / 9.0], [1.0 / 20.0, 1.0 / 11.0, -2.0 / 9.0]]
		rgb_to_xyz_m = rgb_to_xyz(primaries)
		xyz_w = mult_f3_f33([REF_LUMINANCE] * 3, rgb_to_xyz_m)
		y_w = xyz_w[1]
		rgb_w = mult_f3_f33(xyz_w, matrix_16)
		k = 1.0 / (5.0 * L_A + 1.0)
		k4 = k**4
		f_l = 0.2 * k4 * (5.0 * L_A) + 0.1 * (1.0 - k4) ** 2 * (5.0 * L_A) ** (1.0 / 3.0)
		self.f_l_n = f_l / REF_LUMINANCE
		self.cz = MODEL_GAMMA
		d_rgb = [self.f_l_n * y_w / rgb_w[i] for i in range(3)]
		rgb_wc = [d_rgb[i] * rgb_w[i] for i in range(3)]
		rgb_aw = [compress_fwd(v) for v in rgb_wc]
		cone_to_aab = [[CAM_NL_SCALE * v for v in row] for row in base]
		a_w = cone_to_aab[0][0] * rgb_aw[0] + cone_to_aab[1][0] * rgb_aw[1] + cone_to_aab[2][0] * rgb_aw[2]
		self.a_w_j = compress_fwd_unsigned(f_l)
		m1 = mult_f33_f33(rgb_to_xyz_m, matrix_16)
		m2 = [[REF_LUMINANCE if i == j else 0.0 for j in range(3)] for i in range(3)]
		rgb_to_cam16 = mult_f33_f33(m1, m2)
		diag = [[d_rgb[i] if i == j else 0.0 for j in range(3)] for i in range(3)]
		self.rgb_to_cam16_c = mult_f33_f33(rgb_to_cam16, diag)
		self.cam16_c_to_rgb = invert_f33(self.rgb_to_cam16_c)
		self.cone_to_aab = [
			[row[0] / a_w, row[1] * 43.0 * SURROUND[2], row[2] * 43.0 * SURROUND[2]] for row in cone_to_aab
		]
		self.aab_to_cone = invert_f33(self.cone_to_aab)


def rgb_to_aab(rgb, p):
	rgb_m = mult_f3_f33(rgb, p.rgb_to_cam16_c)
	return mult_f3_f33([compress_fwd(v) for v in rgb_m], p.cone_to_aab)


def aab_to_jmh(aab, p):
	if aab[0] <= 0.0:
		return [0.0, 0.0, 0.0]
	j = J_SCALE * aab[0] ** p.cz
	m = math.sqrt(aab[1] ** 2 + aab[2] ** 2)
	h = wrap_to_360(math.degrees(math.atan2(aab[2], aab[1])))
	return [j, m, h]


def rgb_to_jmh(rgb, p):
	return aab_to_jmh(rgb_to_aab(rgb, p), p)


def jmh_to_rgb(jmh, p):
	j, m, h = jmh
	hr = math.radians(h)
	a = (j / J_SCALE) ** (1.0 / p.cz)
	rgb_a = mult_f3_f33([a, m * math.cos(hr), m * math.sin(hr)], p.aab_to_cone)
	return mult_f3_f33([compress_inv(v) for v in rgb_a], p.cam16_c_to_rgb)


def j_to_y(j, p):
	a = (abs(j) / J_SCALE) ** (1.0 / p.cz)
	return compress_inv_unsigned(p.a_w_j * a) / p.f_l_n


def y_to_j(y, p):
	ra = compress_fwd_unsigned(abs(y) * p.f_l_n)
	return copysign(J_SCALE * (ra / p.a_w_j) ** p.cz, y)


class Tonescale:
	"""The Daniele Evo tone scale's constants for one peak luminance, as the reference's `init_TSParams` computes."""

	def __init__(self, n):
		n_r, g, c, c_d, w_g, t_1, r_hit_min, r_hit_max = 100.0, 1.15, 0.18, 10.013, 0.14, 0.04, 128.0, 896.0
		r_hit = r_hit_min + (r_hit_max - r_hit_min) * (math.log(n / n_r) / math.log(10000.0 / 100.0))
		m_0 = n / n_r
		m_1 = 0.5 * (m_0 + math.sqrt(m_0 * (m_0 + 4.0 * t_1)))
		u = ((r_hit / m_1) / ((r_hit / m_1) + 1.0)) ** g
		m = m_1 / u
		w_i = math.log(n / 100.0) / math.log(2.0)
		self.c_t = c_d / n_r * (1.0 + w_i * w_g)
		g_ip = 0.5 * (self.c_t + math.sqrt(self.c_t * (self.c_t + 4.0 * t_1)))
		g_ipp2 = -(m_1 * (g_ip / m) ** (1.0 / g)) / ((g_ip / m) ** (1.0 / g) - 1.0)
		w_2 = c / g_ipp2
		self.s_2 = w_2 * m_1
		self.u_2 = ((r_hit / m_1) / ((r_hit / m_1) + w_2)) ** g
		self.m_2 = m_1 / self.u_2
		self.n_r, self.g, self.t_1 = n_r, g, t_1
		self.forward_limit = 8.0 * r_hit
		self.log_peak = math.log10(n / n_r)

	def forward(self, x):
		f = self.m_2 * (max(0.0, x) / (x + self.s_2)) ** self.g
		return max(0.0, f * f / (f + self.t_1)) * self.n_r


def unit_cube_cusp_corner(corner):
	# Red, yellow, green, cyan, blue, magenta, so the hues rotate in order.
	return [
		1.0 if (corner + 1) % CUSP_CORNER_COUNT < 3 else 0.0,
		1.0 if (corner + 5) % CUSP_CORNER_COUNT < 3 else 0.0,
		1.0 if (corner + 3) % CUSP_CORNER_COUNT < 3 else 0.0,
	]


def cycle_corners(entries):
	"""Rotates the lowest hue to index 1 and wraps one entry onto each end, as the reference's corner tables do."""
	min_index = min(range(CUSP_CORNER_COUNT), key=lambda i: (entries[i][1][2], i))
	rotated = [entries[(i + min_index) % CUSP_CORNER_COUNT] for i in range(CUSP_CORNER_COUNT)]
	cycle = [rotated[-1]] + rotated + [rotated[0]]
	first_rgb, first_jmh = cycle[0]
	last_rgb, last_jmh = cycle[-1]
	cycle[0] = (first_rgb, [first_jmh[0], first_jmh[1], first_jmh[2] - HUE_LIMIT])
	cycle[-1] = (last_rgb, [last_jmh[0], last_jmh[1], last_jmh[2] + HUE_LIMIT])
	return [rgb for rgb, _ in cycle], [jmh for _, jmh in cycle]


class OutputTransform:
	"""The forward ACES 2.0 output transform for one display, with the tables the reference builds at start-up."""

	def __init__(self, peak, limiting_primaries):
		self.peak = peak
		self.input = JMhParams(AP0)
		self.reach = JMhParams(AP1)
		self.limit = JMhParams(limiting_primaries)
		self.ts = Tonescale(peak)
		self.limit_j_max = y_to_j(peak, self.input)
		self.model_gamma_inv = 1.0 / MODEL_GAMMA
		self.reach_m = self.make_reach_m_table()
		self.sat = max(0.2, CHROMA_EXPAND - CHROMA_EXPAND * CHROMA_EXPAND_FACT * self.ts.log_peak)
		self.sat_thr = CHROMA_EXPAND_THR / peak
		self.compr = CHROMA_COMPRESS + CHROMA_COMPRESS * CHROMA_COMPRESS_FACT * self.ts.log_peak
		self.chroma_compress_scale = (0.03379 * peak) ** 0.30596 - 0.45135
		self.mid_j = y_to_j(self.ts.c_t * REF_LUMINANCE, self.input)
		self.focus_dist = FOCUS_DISTANCE + FOCUS_DISTANCE * FOCUS_DISTANCE_SCALING * self.ts.log_peak
		self.lower_hull_gamma_inv = 1.0 / (1.14 + 0.07 * self.ts.log_peak)
		self.cusps = self.make_uniform_hue_gamut_table()
		self.hues = [c[2] for c in self.cusps]
		self.upper_hull_gamma_inv = self.make_upper_hull_gamma_table()
		self.search_range = self.hue_linearity_search_range()

	# --- Table generation ---

	def make_reach_m_table(self):
		table = [0.0] * TOTAL_TABLE_SIZE
		for i in range(TABLE_SIZE):
			hue = i * HUE_LIMIT / TABLE_SIZE
			low, high, outside = 0.0, 50.0, False
			while not outside and high < 1300.0:
				outside = any(v < 0.0 for v in jmh_to_rgb([self.limit_j_max, high, hue], self.reach))
				if not outside:
					low, high = high, high + 50.0
			while high - low > 1e-2:
				sample = (high + low) / 2.0
				if any(v < 0.0 for v in jmh_to_rgb([self.limit_j_max, sample, hue], self.reach)):
					high = sample
				else:
					low = sample
			table[i + BASE_INDEX] = high
		table[0] = table[TABLE_SIZE]
		table[BASE_INDEX + TABLE_SIZE] = table[BASE_INDEX]
		return table

	def reach_corners(self):
		limit_a = (self.limit_j_max / J_SCALE) ** (1.0 / self.reach.cz)
		entries = []
		for i in range(CUSP_CORNER_COUNT):
			rgb = unit_cube_cusp_corner(i)
			lower, upper = 0.0, self.ts.forward_limit
			while upper - lower > REACH_CUSP_TOLERANCE:
				test = (lower + upper) / 2.0
				if rgb_to_aab([test * v for v in rgb], self.reach)[0] < limit_a:
					lower = test
				else:
					upper = test
			entries.append((None, rgb_to_jmh([upper * v for v in rgb], self.reach)))
		return cycle_corners(entries)[1]

	def limiting_corners(self):
		entries = []
		for i in range(CUSP_CORNER_COUNT):
			rgb = [self.peak / REF_LUMINANCE * v for v in unit_cube_cusp_corner(i)]
			entries.append((rgb, rgb_to_jmh(rgb, self.limit)))
		return cycle_corners(entries)

	@staticmethod
	def sorted_cube_hues(reach_jmh, limit_jmh):
		hues = []
		reach_idx = limit_idx = 1
		while reach_idx < CUSP_CORNER_COUNT + 1 or limit_idx < CUSP_CORNER_COUNT + 1:
			reach_hue = reach_jmh[reach_idx][2]
			limit_hue = limit_jmh[limit_idx][2]
			if reach_hue == limit_hue:
				hues.append(reach_hue)
				reach_idx += 1
				limit_idx += 1
			elif reach_hue < limit_hue:
				hues.append(reach_hue)
				reach_idx += 1
			else:
				hues.append(limit_hue)
				limit_idx += 1
		assert len(hues) == MAX_SORTED_CORNERS, "The reach and limiting gamuts share a corner hue."
		return hues

	@staticmethod
	def hue_table(sorted_hues):
		ideal_spacing = TABLE_SIZE / HUE_LIMIT
		samples_count = [0] * (MAX_SORTED_CORNERS + 2)
		last_idx = 0
		min_index = 0 if sorted_hues[0] == 0.0 else 1
		for hue_idx in range(MAX_SORTED_CORNERS):
			nominal_idx = min(max(ctl_round(sorted_hues[hue_idx] * ideal_spacing), min_index), TABLE_SIZE - 1)
			if last_idx == nominal_idx:
				if hue_idx > 1 and samples_count[hue_idx - 2] != samples_count[hue_idx - 1] - 1:
					samples_count[hue_idx - 1] -= 1
				else:
					nominal_idx += 1
			samples_count[hue_idx] = min(nominal_idx, TABLE_SIZE - 1)
			min_index = nominal_idx
			last_idx = min_index

		table = [0.0] * TOTAL_TABLE_SIZE

		def interval(samples, lower, upper, base):
			delta = (upper - lower) / samples
			for i in range(samples):
				table[base + i] = lower + i * delta

		total = 0
		interval(samples_count[0], 0.0, sorted_hues[0], total + 1)
		total += samples_count[0]
		for i in range(1, MAX_SORTED_CORNERS):
			samples = samples_count[i] - samples_count[i - 1]
			interval(samples, sorted_hues[i - 1], sorted_hues[i], total + 1)
			total += samples
		interval(TABLE_SIZE - total, sorted_hues[MAX_SORTED_CORNERS - 1], HUE_LIMIT, total + 1)
		table[0] = table[BASE_INDEX + TABLE_SIZE - 1] - HUE_LIMIT
		table[BASE_INDEX + TABLE_SIZE] = table[BASE_INDEX] + HUE_LIMIT
		return table

	def display_cusp_for_hue(self, hue, rgb_corners, jmh_corners):
		upper_corner = 1
		for i in range(1, CUSP_CORNER_COUNT + 2):
			if jmh_corners[i][2] > hue:
				upper_corner = i
				break
		lower_corner = upper_corner - 1
		if jmh_corners[lower_corner][2] == hue:
			return jmh_corners[lower_corner][0], jmh_corners[lower_corner][1]
		cusp_lower, cusp_upper = rgb_corners[lower_corner], rgb_corners[upper_corner]
		lower_t, upper_t = 0.0, 1.0
		while upper_t - lower_t > DISPLAY_CUSP_TOLERANCE:
			sample_t = (lower_t + upper_t) / 2.0
			jmh = rgb_to_jmh([lerp(a, b, sample_t) for a, b in zip(cusp_lower, cusp_upper)], self.limit)
			if jmh[2] < jmh_corners[lower_corner][2]:
				upper_t = sample_t
			elif jmh[2] >= jmh_corners[upper_corner][2]:
				lower_t = sample_t
			elif jmh[2] > hue:
				upper_t = sample_t
			else:
				lower_t = sample_t
		sample_t = (lower_t + upper_t) / 2.0
		jmh = rgb_to_jmh([lerp(a, b, sample_t) for a, b in zip(cusp_lower, cusp_upper)], self.limit)
		return jmh[0], jmh[1]

	def make_uniform_hue_gamut_table(self):
		reach_jmh = self.reach_corners()
		limit_rgb, limit_jmh = self.limiting_corners()
		hues = self.hue_table(self.sorted_cube_hues(reach_jmh, limit_jmh))
		table = [[0.0, 0.0, 0.0] for _ in range(TOTAL_TABLE_SIZE)]
		for i in range(BASE_INDEX, TOTAL_TABLE_SIZE):
			j, m = self.display_cusp_for_hue(hues[i], limit_rgb, limit_jmh)
			table[i] = [j, m * (1.0 + SMOOTH_M * SMOOTH_CUSPS), hues[i]]
		table[0] = [table[TABLE_SIZE][0], table[TABLE_SIZE][1], hues[0]]
		table[BASE_INDEX + TABLE_SIZE] = [table[BASE_INDEX][0], table[BASE_INDEX][1], hues[BASE_INDEX + TABLE_SIZE]]
		return table

	def gamma_fits(self, cusp, tests, top_gamma_inv):
		luminance_limit = self.peak / REF_LUMINANCE
		for hue, j_intersect_source, slope, j_intersect_cusp in tests:
			m = find_gamut_boundary_intersection(
				cusp, self.limit_j_max, top_gamma_inv, self.lower_hull_gamma_inv, j_intersect_source, slope, j_intersect_cusp
			)
			rgb = jmh_to_rgb([j_intersect_source + slope * m, m, hue], self.limit)
			if not any(v > luminance_limit for v in rgb):
				return False
		return True

	def make_upper_hull_gamma_table(self):
		table = [0.0] * TOTAL_TABLE_SIZE
		for i in range(BASE_INDEX, BASE_INDEX + TABLE_SIZE):
			j_cusp, m_cusp, hue = self.cusps[i]
			cusp = (j_cusp, m_cusp)
			analytical_threshold = lerp(j_cusp, self.limit_j_max, FOCUS_GAIN_BLEND)
			focus_j = compute_focus_j(j_cusp, self.mid_j, self.limit_j_max)
			tests = []
			for position in (0.01, 0.1, 0.5, 0.8, 0.99):
				test_j = lerp(j_cusp, self.limit_j_max, position)
				gain = focus_gain(test_j, analytical_threshold, self.limit_j_max, self.focus_dist)
				j_intersect = solve_j_intersect(test_j, m_cusp, focus_j, self.limit_j_max, gain)
				slope = compression_vector_slope(j_intersect, focus_j, self.limit_j_max, gain)
				j_intersect_cusp = solve_j_intersect(j_cusp, m_cusp, focus_j, self.limit_j_max, gain)
				tests.append((hue, j_intersect, slope, j_intersect_cusp))
			low, high, outside = GAMMA_MINIMUM, GAMMA_MINIMUM + GAMMA_SEARCH_STEP, False
			while not outside and high < GAMMA_MAXIMUM:
				if self.gamma_fits(cusp, tests, 1.0 / high):
					outside = True
				else:
					low, high = high, high + GAMMA_SEARCH_STEP
			while high - low > GAMMA_ACCURACY:
				test_gamma = (high + low) / 2.0
				if self.gamma_fits(cusp, tests, 1.0 / test_gamma):
					high = test_gamma
				else:
					low = test_gamma
			table[i] = 1.0 / high
		table[0] = table[TABLE_SIZE]
		table[TABLE_SIZE + BASE_INDEX] = table[BASE_INDEX]
		return table

	def hue_linearity_search_range(self):
		lower, upper = 0, 1
		for i in range(BASE_INDEX, BASE_INDEX + TABLE_SIZE):
			delta = i - hue_position_in_uniform_table(self.hues[i], TOTAL_TABLE_SIZE)
			lower = min(lower, delta)
			upper = max(upper, delta + 1)
		return lower, upper

	# --- Per-pixel transform ---

	def reach_m_at(self, h):
		base = hue_position_in_uniform_table(h, TABLE_SIZE)
		t = h - base
		return lerp(self.reach_m[base + BASE_INDEX], self.reach_m[base + BASE_INDEX + 1], t)

	def hue_interval(self, h):
		i = BASE_INDEX + hue_position_in_uniform_table(h, TOTAL_TABLE_SIZE)
		i_lo = max(BASE_INDEX, i + self.search_range[0])
		i_hi = min(BASE_INDEX + TABLE_SIZE, i + self.search_range[1])
		while i_lo + 1 < i_hi:
			if h > self.hues[i]:
				i_lo = i
			else:
				i_hi = i
			i = (i_lo + i_hi) // 2
		return max(1, i_hi)

	def hue_interval_bounded(self, h, steps):
		# The reference's narrowed search with a fixed number of steps, as the shader runs it without `while`.
		i = BASE_INDEX + hue_position_in_uniform_table(h, TOTAL_TABLE_SIZE)
		i_lo = max(BASE_INDEX, i + self.search_range[0])
		i_hi = min(BASE_INDEX + TABLE_SIZE, i + self.search_range[1])
		for _ in range(steps):
			if i_lo + 1 >= i_hi:
				break
			if h > self.hues[i]:
				i_lo = i
			else:
				i_hi = i
			i = (i_lo + i_hi) // 2
		return max(1, i_hi)

	def cusp_window(self):
		"""Returns the offsets from 1 + floor(h) that bound the entry whose interval holds h.

		The entry k holds the hues in (hues[k - 1], hues[k]], so floor(h) there runs from floor(hues[k - 1]) to
		floor(hues[k]). Taking every k gives a window that always contains the answer, which lets the shader scan a
		few entries in place of the reference's full bisection.
		"""
		lower, upper = 0, 0
		for k in range(BASE_INDEX, BASE_INDEX + TABLE_SIZE + 1):
			for whole in range(max(0, math.floor(self.hues[k - 1])), min(359, math.floor(self.hues[k])) + 1):
				lower = min(lower, k - (1 + whole))
				upper = max(upper, k - (1 + whole))
		return lower, upper

	def cusp_interval_in_window(self, h, window):
		# The first entry of the window whose hue reaches h, as the shader scans it.
		start = 1 + math.floor(h) + window[0]
		for k in range(start, start + window[1] - window[0] + 1):
			if self.hues[k] >= h:
				return k
		return start + window[1] - window[0]

	def cusp_interval(self, h):
		# The reference's own search for the cusp, over the whole table.
		low_i, high_i = 0, BASE_INDEX + TABLE_SIZE
		i = hue_position_in_uniform_table(h, TABLE_SIZE) + BASE_INDEX
		while low_i + 1 < high_i:
			if h > self.cusps[i][2]:
				low_i = i
			else:
				high_i = i
			i = (low_i + high_i) // 2
		return high_i

	def chroma_compress(self, jmh, tonemapped_j):
		j, m, h = jmh
		if m == 0.0:
			return [tonemapped_j, m, h]
		nj = tonemapped_j / self.limit_j_max
		snj = max(0.0, 1.0 - nj)
		m_norm = chroma_compress_norm(h, self.chroma_compress_scale)
		limit = nj**self.model_gamma_inv * self.reach_m_at(h) / m_norm
		m_compr = m * (tonemapped_j / j) ** self.model_gamma_inv / m_norm
		m_compr = limit - toe(limit - m_compr, limit - 0.001, snj * self.sat, math.sqrt(nj * nj + self.sat_thr))
		m_compr = toe(m_compr, limit, nj * self.compr, snj)
		return [tonemapped_j, m_compr * m_norm, h]

	def gamut_compress(self, jmh):
		j, m, h = jmh
		if j <= 0.0:
			return [0.0, 0.0, h]
		if m < 0.0 or j > self.limit_j_max:
			return [j, 0.0, h]
		# The reference finds the cusp's interval with a full search, and the gamma's with a narrowed one that starts
		# from a position offset by the base index twice, so it lands one entry high for about a quarter of hues. The
		# gamma then extrapolates slightly from that interval. Both follow the reference.
		i_cusp = self.cusp_interval(h)
		lo, hi = self.cusps[i_cusp - 1], self.cusps[i_cusp]
		t = (h - lo[2]) / (hi[2] - lo[2])
		cusp = (lerp(lo[0], hi[0], t), lerp(lo[1], hi[1], t))
		# The reference weighs the gamma by the hue difference itself, not by the normalized position.
		i_gamma = self.hue_interval(h)
		gamma_top_inv = lerp(
			self.upper_hull_gamma_inv[i_gamma - 1], self.upper_hull_gamma_inv[i_gamma], h - self.hues[i_gamma - 1]
		)
		focus_j = compute_focus_j(cusp[0], self.mid_j, self.limit_j_max)
		analytical_threshold = lerp(cusp[0], self.limit_j_max, FOCUS_GAIN_BLEND)
		gain = focus_gain(j, analytical_threshold, self.limit_j_max, self.focus_dist)
		j_intersect_source = solve_j_intersect(j, m, focus_j, self.limit_j_max, gain)
		slope = compression_vector_slope(j_intersect_source, focus_j, self.limit_j_max, gain)
		j_intersect_cusp = solve_j_intersect(cusp[0], cusp[1], focus_j, self.limit_j_max, gain)
		boundary_m = find_gamut_boundary_intersection(
			cusp, self.limit_j_max, gamma_top_inv, self.lower_hull_gamma_inv, j_intersect_source, slope, j_intersect_cusp
		)
		if boundary_m <= 0.0:
			return [j, 0.0, h]
		reach_boundary_m = line_and_boundary_intersection_m(
			j_intersect_source, slope, self.model_gamma_inv, self.limit_j_max, self.reach_m_at(h), self.limit_j_max
		)
		remapped_m = remap_m(m, boundary_m, reach_boundary_m)
		return [j_intersect_source + remapped_m * slope, remapped_m, h]

	def forward_ap0(self, aces):
		ap1 = [min(max(v, 0.0), self.ts.forward_limit) for v in mult_f3_f33(aces, AP0_TO_AP1)]
		jmh = rgb_to_jmh(mult_f3_f33(ap1, AP1_TO_AP0), self.input)
		linear = j_to_y(jmh[0], self.input) / REF_LUMINANCE
		j_ts = y_to_j(self.ts.forward(linear), self.input)
		jmh = self.chroma_compress(jmh, j_ts)
		jmh = self.gamut_compress(jmh)
		return [min(max(v, 0.0), self.peak / REF_LUMINANCE) for v in jmh_to_rgb(jmh, self.limit)]


def chroma_compress_norm(h, scale):
	hr = math.radians(h)
	a, b = math.cos(hr), math.sin(hr)
	cos2, sin2 = a * a - b * b, 2.0 * a * b
	cos3, sin3 = 4.0 * a * a * a - 3.0 * a, 3.0 * b - 4.0 * b * b * b
	m = 11.34072 * a + 16.46899 * cos2 + 7.88380 * cos3 + 14.66441 * b - 6.37224 * sin2 + 9.19364 * sin3 + 77.12896
	return m * scale


def toe(x, limit, k1_in, k2_in):
	if x > limit:
		return x
	k2 = max(k2_in, 0.001)
	k1 = math.sqrt(k1_in * k1_in + k2 * k2)
	k3 = (limit + k1) / (limit + k2)
	minus_b = k3 * x - k1
	minus_c = k2 * k3 * x
	return 0.5 * (minus_b + math.sqrt(minus_b * minus_b + 4.0 * minus_c))


def compute_focus_j(cusp_j, mid_j, limit_j_max):
	return lerp(cusp_j, mid_j, min(1.0, CUSP_MID_BLEND - cusp_j / limit_j_max))


def focus_gain(j, analytical_threshold, limit_j_max, focus_dist):
	gain = limit_j_max * focus_dist
	if j > analytical_threshold:
		adjustment = math.log10((limit_j_max - analytical_threshold) / max(0.0001, limit_j_max - j))
		gain *= adjustment * adjustment + 1.0
	return gain


def solve_j_intersect(j, m, focus_j, max_j, slope_gain):
	m_scaled = m / slope_gain
	a = m_scaled / focus_j
	if j < focus_j:
		b = 1.0 - m_scaled
		c = -j
		return -2.0 * c / (b + math.sqrt(b * b - 4.0 * a * c))
	b = -(1.0 + m_scaled + max_j * a)
	c = max_j * m_scaled + j
	return -2.0 * c / (b - math.sqrt(b * b - 4.0 * a * c))


def compression_vector_slope(intersect_j, focus_j, limit_j_max, slope_gain):
	direction = intersect_j if intersect_j < focus_j else limit_j_max - intersect_j
	return direction * (intersect_j - focus_j) / (focus_j * slope_gain)


def line_and_boundary_intersection_m(j_axis_intersect, slope, inv_gamma, j_max, m_max, j_reference):
	shifted = j_reference * (j_axis_intersect / j_reference) ** inv_gamma
	return shifted * m_max / (j_max - slope * m_max)


def smin_scaled(a, b, scale_reference):
	s = SMOOTH_CUSPS * scale_reference
	h = max(s - abs(a - b), 0.0) / s
	return min(a, b) - h * h * h * s / 6.0


def find_gamut_boundary_intersection(cusp, j_max, gamma_top_inv, gamma_bottom_inv, j_source, slope, j_cusp):
	lower = line_and_boundary_intersection_m(j_source, slope, gamma_bottom_inv, cusp[0], cusp[1], j_cusp)
	upper = line_and_boundary_intersection_m(j_max - j_source, -slope, gamma_top_inv, j_max - cusp[0], cusp[1], j_max - j_cusp)
	return smin_scaled(lower, upper, cusp[1])


def remap_m(m, gamut_boundary_m, reach_boundary_m):
	proportion = max(gamut_boundary_m / reach_boundary_m, COMPRESSION_THRESHOLD)
	threshold = proportion * gamut_boundary_m
	if m <= threshold or proportion >= 1.0:
		return m
	gamut_offset = gamut_boundary_m - threshold
	reach_offset = reach_boundary_m - threshold
	scale = reach_offset / (reach_offset / gamut_offset - 1.0)
	nd = (m - threshold) / scale
	return threshold + scale * nd / (1.0 + nd)


# --- The engine's side of the transform ---

# The renderer's scene color is linear sRGB with a D65 white. This is the D65-to-D60-adapted conversion to ACEScg that
# the engine's ACES color-grading shaders share.
LINEAR_SRGB_TO_ACESCG = [
	[0.6130974024, 0.3395231462, 0.0473794514],
	[0.0701937225, 0.9163538791, 0.0134523985],
	[0.0206155929, 0.1095697729, 0.8698146342],
]


def srgb_encode(v):
	return 12.92 * v if v <= 0.0031308 else 1.055 * v ** (1.0 / 2.4) - 0.055


def scene_srgb_to_display(transform, rgb):
	"""Renders a scene-linear sRGB color as an sRGB-encoded display color, with no look in between."""
	acescg = [sum(LINEAR_SRGB_TO_ACESCG[r][c] * rgb[c] for c in range(3)) for r in range(3)]
	aces = mult_f3_f33(acescg, AP1_TO_AP0)
	return [srgb_encode(v) for v in transform.forward_ap0(aces)]


# --- BESL output ---


def number(x):
	# Ten significant digits, more than f32 holds, written without an exponent because BESL has no scientific notation.
	decimals = 10 if x == 0.0 else max(1, 10 - 1 - math.floor(math.log10(abs(x))))
	text = f"{x:.{decimals}f}".rstrip("0")
	return text + "0" if text.endswith(".") else text


def signed_term(coefficient, variable, first):
	if first:
		return f"{number(coefficient)} * {variable}"
	sign = "-" if coefficient < 0.0 else "+"
	return f" {sign} {number(abs(coefficient))} * {variable}"


def matrix_function(name, comment, m):
	# A row vector times `m` is one dot product per output column.
	lines = [f"// {comment}", f"{name}: fn(v: vec3f) -> vec3f {{", "\treturn vec3f("]
	for j in range(3):
		separator = "," if j < 2 else ""
		terms = "".join(signed_term(m[i][j], variable, i == 0) for i, variable in enumerate(("v.x", "v.y", "v.z")))
		lines.append(f"\t\t{terms}{separator}")
	lines += ["\t);", "}", ""]
	return "\n".join(lines)


def table(name, comment, values):
	lines = [f"// {comment}", f"{name}: const f32[{len(values)}] = f32[{len(values)}]("]
	for start in range(0, len(values), 8):
		chunk = ", ".join(number(v) for v in values[start : start + 8])
		lines.append(f"\t{chunk}{',' if start + 8 < len(values) else ''}")
	lines += [");", ""]
	return "\n".join(lines)


def scalar(name, comment, value, kind="f32"):
	text = str(value) if kind in ("i32", "u32") else number(value)
	return f"// {comment}\n{name}: const {kind} = {text};\n"


def hue_search_steps(t):
	# Bisection halves the interval each step after the first probe, which starts off center.
	return max(1, math.ceil(math.log2(t.search_range[1] - t.search_range[0])) + 1)


def besl(t):
	ap1_to_cam = mult_f33_f33(AP1_TO_AP0, t.input.rgb_to_cam16_c)
	steps = hue_search_steps(t)
	window = t.cusp_window()
	# The shader indexes the tables with u32 arithmetic, which a negative offset would wrap.
	assert t.search_range[0] >= 0 and window[0] >= 0, "A hue search starts below its uniform position; use signed offsets."
	parts = [
		"// Generated by scripts/generate-aces2-tables.py. Do not edit; change the script and run it again.",
		"//",
		"// The ACES 2.0 output transform's constants and per-hue tables for a Rec.709 display with a D65 white and a",
		"// 100-nit peak, computed offline from the Academy's Apache-2.0 reference implementation",
		"// (https://github.com/aces-aswf/aces-core), so the shader that reads them builds no tables at run time.",
		"",
		matrix_function(
			"aces2_acescg_to_cam16",
			"Converts ACEScg to the CAM's adapted cone space, through ACES2065-1, as the transform's input conversion does.",
			ap1_to_cam,
		),
		matrix_function("aces2_input_cone_to_aab", "Converts compressed cone responses to the input space's achromatic and opponent signals.", t.input.cone_to_aab),
		matrix_function("aces2_limit_aab_to_cone", "Converts achromatic and opponent signals back to the display's compressed cone responses.", t.limit.aab_to_cone),
		matrix_function("aces2_limit_cam16_to_rgb", "Converts the display's adapted cone responses to linear Rec.709 display color.", t.limit.cam16_c_to_rgb),
		scalar("ACES2_CAM_NL_OFFSET", "The offset of the CAM's cone response compression, which sets where it saturates.", CAM_NL_OFFSET),
		scalar("ACES2_PEAK", "The display's peak, relative to the 100-nit reference white.", t.peak / REF_LUMINANCE),
		scalar("ACES2_SMOOTH_CUSPS", "How far the smooth minimum rounds the gamut's cusp, relative to the cusp's colorfulness.", SMOOTH_CUSPS),
		scalar("ACES2_CUSP_MID_BLEND", "How far the gamut compression's focus moves from the cusp toward mid grey.", CUSP_MID_BLEND),
		scalar("ACES2_FOCUS_GAIN_BLEND", "Where between the cusp and peak lightness the focus gain starts to grow.", FOCUS_GAIN_BLEND),
		scalar("ACES2_COMPRESSION_THRESHOLD", "The share of the gamut boundary below which colorfulness stays uncompressed.", COMPRESSION_THRESHOLD),
		scalar("ACES2_F_L_N", "The input space's luminance-level adaptation factor over the reference luminance.", t.input.f_l_n),
		scalar("ACES2_CZ", "The CAM's exponent from achromatic response to lightness J.", t.input.cz),
		scalar("ACES2_A_W_J", "The input space's achromatic response of the reference white.", t.input.a_w_j),
		scalar("ACES2_TONESCALE_S_2", "The tone scale's scene-referred offset.", t.ts.s_2),
		scalar("ACES2_TONESCALE_M_2", "The tone scale's output scale.", t.ts.m_2),
		scalar("ACES2_TONESCALE_G", "The tone scale's contrast exponent.", t.ts.g),
		scalar("ACES2_TONESCALE_T_1", "The tone scale's flare compensation.", t.ts.t_1),
		scalar("ACES2_FORWARD_LIMIT", "The largest scene value the transform accepts in any ACEScg channel.", t.ts.forward_limit),
		scalar("ACES2_LIMIT_J_MAX", "The lightness of the display's peak white.", t.limit_j_max),
		scalar("ACES2_MODEL_GAMMA_INV", "One over the CAM's model gamma, which keeps M over J constant as J changes.", t.model_gamma_inv),
		scalar("ACES2_SAT", "The chroma expansion's strength.", t.sat),
		scalar("ACES2_SAT_THR", "The chroma expansion's threshold near black.", t.sat_thr),
		scalar("ACES2_COMPR", "The chroma compression's strength.", t.compr),
		scalar("ACES2_CHROMA_COMPRESS_SCALE", "The scale of the chroma compression's hue-dependent normalization.", t.chroma_compress_scale),
		scalar("ACES2_MID_J", "The lightness the display shows mid grey at.", t.mid_j),
		scalar("ACES2_FOCUS_DIST", "The gamut compression's focus distance.", t.focus_dist),
		scalar("ACES2_LOWER_HULL_GAMMA_INV", "One over the gamma of the gamut's lower hull.", t.lower_hull_gamma_inv),
		scalar("ACES2_HUE_SEARCH_LOWER", "How far above its uniform position a hue's table interval can start.", t.search_range[0], "u32"),
		scalar("ACES2_HUE_SEARCH_UPPER", "How far above its uniform position a hue's table interval can end.", t.search_range[1], "u32"),
		scalar("ACES2_HUE_SEARCH_STEPS", "The bisection steps that always close the narrowed hue search.", steps, "u32"),
		scalar("ACES2_CUSP_WINDOW_START", "The offset from 1 + floor(h) of the first table entry that can hold h's cusp.", window[0], "u32"),
		scalar("ACES2_CUSP_WINDOW_SIZE", "How many table entries, from that first one, can hold h's cusp.", window[1] - window[0] + 1, "u32"),
		table("ACES2_REACH_M", "The reach gamut's colorfulness at the peak lightness, at every whole degree of hue.", t.reach_m),
		table("ACES2_HUES", "The hue of every gamut table entry, nearly uniform but landing on the gamuts' corners.", t.hues),
		table("ACES2_CUSP_J", "The lightness of the display gamut's cusp at each table hue.", [c[0] for c in t.cusps]),
		table("ACES2_CUSP_M", "The colorfulness of the display gamut's cusp at each table hue, widened for the smooth minimum.", [c[1] for c in t.cusps]),
		table("ACES2_UPPER_HULL_GAMMA_INV", "One over the gamma of the gamut's upper hull at each table hue.", t.upper_hull_gamma_inv),
	]
	return "\n".join(parts)


def main():
	transform = OutputTransform(PEAK_LUMINANCE, REC709)
	if "--vectors" in sys.argv:
		for rgb in (
			[0.18, 0.18, 0.18],
			[0.0, 0.0, 0.0],
			[1.0, 1.0, 1.0],
			[16.0, 16.0, 16.0],
			[0.5, 0.1, 0.05],
			[0.05, 0.4, 0.1],
			[0.02, 0.05, 0.6],
			[4.0, 0.5, 0.1],
			[0.3, 0.25, 0.9],
			[0.9, 0.9, 0.05],
		):
			print(rgb, [round(v, 7) for v in scene_srgb_to_display(transform, rgb)])
		# The shader's bounded searches must agree with the reference's loops at every hue.
		window = transform.cusp_window()
		for step in range(360000):
			h = step / 1000.0
			assert transform.hue_interval_bounded(h, hue_search_steps(transform)) == transform.hue_interval(h), h
			assert transform.cusp_interval_in_window(h, window) == transform.cusp_interval(h), h
		return
	sys.stdout.write(besl(transform))


if __name__ == "__main__":
	main()
