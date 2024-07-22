static MIN_BLOCK_LENGTH: usize = 10000;
static GDEFLATE_PAGE_SIZE: usize = 65536;
static OUTPUT_END_PADDING: usize = 8;
const NUM_STREAMS: usize = 32;
static BITS_PER_PACKET: usize = 32;

static SUPPORT_NEAR_OPTIMAL_PARSING: bool = true;

static MATCHFINDER_MEM_ALIGNMENT: usize = 32;

struct Compressor {
	libdeflater_compressor: libdeflater::Compressor,
	compression_level: u32,
	method: u32,
	min_size_to_compress: usize,
	max_search_depth: usize,
	nice_match_length: usize,
	num_optim_passes: usize,
}

impl Compressor {
	pub fn new(compression_level: u32) -> Self {
		Compressor {
			libdeflater_compressor: libdeflater::Compressor::new(libdeflater::CompressionLvl::new(compression_level as i32).unwrap()),
			compression_level,
			method: 0,
			min_size_to_compress: 0,
			max_search_depth: 0,
			nice_match_length: 0,
			num_optim_passes: 0,
		}
	}
}

fn libdeflate_alloc_gdeflate_compressor(compression_level: u32) -> Compressor {
	if compression_level < 0 || compression_level > 12 { panic!() }

	let size = if SUPPORT_NEAR_OPTIMAL_PARSING {
		if compression_level >= 8 {
			1
		} else {
			if compression_level >= 1 {
				0
			} else {
				0
			}
		}
	} else {
		if compression_level >= 1 {
			0
		} else {
			0
		}
	};

	let mut c = Compressor::new(compression_level);

	/*
	 * The higher the compression level, the more we should bother trying to
	 * compress very small inputs.
	 */
	c.min_size_to_compress = 56 as usize - (compression_level as usize * 4 as usize);

	let method;
	let max_search_depth;
	let nice_match_length;
	let num_optim_passes;

	match compression_level {
		0 => {
			method = 0;
			max_search_depth = 0;
			nice_match_length = 0;
			num_optim_passes = 0;
		},
		1 => {
			method = 1;
			max_search_depth = 2;
			nice_match_length = 8;
			num_optim_passes = 0;
		},
		2 => {
			method = 1;
			max_search_depth = 6;
			nice_match_length = 10;
			num_optim_passes = 0;
		},
		3 => {
			method = 1;
			max_search_depth = 12;
			nice_match_length = 14;
			num_optim_passes = 0;
		},
		4 => {
			method = 1;
			max_search_depth = 24;
			nice_match_length = 24;
			num_optim_passes = 0;
		},
		5 => {
			method = 2;
			max_search_depth = 20;
			nice_match_length = 30;
			num_optim_passes = 0;
		},
		6 => {
			method = 2;
			max_search_depth = 40;
			nice_match_length = 65;
			num_optim_passes = 0;
		},
		7 => {
			method = 2;
			max_search_depth = 100;
			nice_match_length = 130;
			num_optim_passes = 0;
		},
		8 => {
			method = 3; // Near optimal
			max_search_depth = 12;
			nice_match_length = 20;
			num_optim_passes = 1;
		},
		9 => {
			method = 3; // Near optimal
			max_search_depth = 16;
			nice_match_length = 26;
			num_optim_passes = 2;
		},
		10 => {
			method = 3; // Near optimal
			max_search_depth = 30;
			nice_match_length = 50;
			num_optim_passes = 2;
		},
		11 => {
			method = 3; // Near optimal
			max_search_depth = 60;
			nice_match_length = 80;
			num_optim_passes = 3;
		},
		_ => {
			method = 3; // Near optimal
			max_search_depth = 100;
			nice_match_length = 133;
			num_optim_passes = 4;
		}
	}

	c.method = method;
	c.max_search_depth = max_search_depth;
	c.nice_match_length = nice_match_length;
	c.num_optim_passes = num_optim_passes;

	// deflate_init_offset_slot_fast(c);
	// deflate_init_static_codes(c);
	// deflate_init_length_slot();

	return c;
}

/// Returns the maximum number of bytes that libdeflate_gdeflate_compress() may write to out_pages.
/// This is an upper bound on the size of the compressed data, but the actual size may be smaller.
/// The caller must ensure that out_pages is large enough to hold the compressed data.
/// This number WILL be larger or equal to the size of the uncompressed data.
fn libdeflate_gdeflate_compress_bound(_compressor: &Compressor, in_nbytes: usize, out_npages: &mut usize) -> usize {
	/*
	 * The worst case is all uncompressed blocks where one block has length
	 * <= MIN_BLOCK_LENGTH and the others have length MIN_BLOCK_LENGTH.
	 * Each uncompressed block has 5 bytes of overhead: 1 for BFINAL, BTYPE,
	 * and alignment to a byte boundary; 2 for LEN; and 2 for NLEN.
	 */
	let max_num_blocks = std::cmp::max(std::primitive::usize::div_ceil(in_nbytes, MIN_BLOCK_LENGTH), 1);
	let npages = in_nbytes.div_ceil(GDEFLATE_PAGE_SIZE);

	*out_npages = npages;

	((5 * max_num_blocks) + GDEFLATE_PAGE_SIZE + 1 + OUTPUT_END_PADDING + (NUM_STREAMS * BITS_PER_PACKET) / 8) * npages
}

fn gdeflate_compress(c: &mut Compressor, in_bytes: &[u8], out_pages: &[&[u8]]) -> usize {
	let mut out_nbytes = 0;
	let mut npages = out_pages.len();
	let	upper_bound = libdeflate_gdeflate_compress_bound(c, in_bytes.len(), &mut npages);
	let page_upper_bound = upper_bound / npages;
	
	if out_pages.len() != npages { return 0 as usize }
	
	for page in 0..npages {
		let comp_page_nbytes;
		let page_nbytes = if in_bytes.len() > GDEFLATE_PAGE_SIZE { GDEFLATE_PAGE_SIZE } else { in_bytes.len() };

		if out_pages[page].len() < page_upper_bound { return 0 as usize }

		let mut out_deflate_data = Vec::with_capacity(page_upper_bound);

		unsafe {
			out_deflate_data.set_len(page_upper_bound);
		}

		comp_page_nbytes = match c.method {
			0 => { // No compression
				return 0 as usize;
			},
			1 => { // Greedy matching
				c.libdeflater_compressor.deflate_compress(&in_bytes[page*page_nbytes..page*page_nbytes+page_nbytes], out_deflate_data.as_mut_slice()).unwrap()
			},
			2 => { // Lazy matching
				c.libdeflater_compressor.deflate_compress(&in_bytes[page*page_nbytes..page*page_nbytes+page_nbytes], out_deflate_data.as_mut_slice()).unwrap()
			},
			3 => { // Near optimal
				c.libdeflater_compressor.deflate_compress(&in_bytes[page*page_nbytes..page*page_nbytes+page_nbytes], out_deflate_data.as_mut_slice()).unwrap()
			},
			_ => { return 0 as usize; }
		};

		//comp_page_nbytes = (*c->impl)(c, in_bytes, page_nbytes, out_pages[page].data, page_upper_bound);

		//out_pages[page].len() = comp_page_nbytes;

		/* Page did not fit - bail out. */
		if comp_page_nbytes == 0 { return 0 as usize }

		out_nbytes += comp_page_nbytes;
	}

	out_nbytes
}

/*
 * The main DEFLATE decompressor structure.  Since this implementation only
 * supports full buffer decompression, this structure does not store the entire
 * decompression state, but rather only some arrays that are too large to
 * comfortably allocate on the stack.
 */


const DEFLATE_MAX_NUM_SYMS: usize = 288;
const LITLEN_ENOUGH: usize = 1334;
const PRECODE_ENOUGH: usize = 128;

struct L {
	lens: [u8; DEFLATE_NUM_LITLEN_SYMS + DEFLATE_NUM_OFFSET_SYMS + DEFLATE_MAX_LENS_OVERRUN],
	precode_decode_table: [u32; PRECODE_ENOUGH],
}

struct U {
	precode_lens: [u8; DEFLATE_NUM_PRECODE_SYMS],
	l: L,
	litlen_decode_table: [u32; LITLEN_ENOUGH],
}

struct Decompressor {
	libdefalter_decompressor: libdeflater::Decompressor,
	offset_decode_table: [u32; OFFSET_ENOUGH],
	/* used only during build_decode_table() */
	sorted_syms: [u16; DEFLATE_MAX_NUM_SYMS],
	u: U,
	static_codes_loaded: bool,
}

/*
 * GDeflate deferred copy state structure.
 */
#[derive(Copy, Clone)]
struct gdeflate_deferred_copy {
	length: u32,
	out_next: *mut u32,
}

/*
 * GDeflate state structure.
 */
struct gdeflate_state {
	bitbuf: [u64; NUM_STREAMS],
	bitsleft: [u32; NUM_STREAMS],
	copies: [gdeflate_deferred_copy; NUM_STREAMS],
	idx: usize,
}

impl gdeflate_state {
	pub fn new() -> Self {
		gdeflate_state {
			bitbuf: [0; NUM_STREAMS],
			bitsleft: [0; NUM_STREAMS],
			copies: [gdeflate_deferred_copy { length: 0, out_next: 0 as *mut u32 }; NUM_STREAMS],
			idx: 0,
		}
	}

	/*
	* Does the bitbuffer variable currently contain at least 'n' bits?
	*/
	pub fn HAVE_BITS(&self, n: u32) -> bool { self.bitsleft[self.idx as usize] >= n }

	/*
	* Does the bitbuffer variable currently contain at least 'n' bits?
	*/
	pub fn BITS(&self, n: usize) -> usize { (self.bitbuf[self.idx as usize] & (((1u64 << n) - 1) as u64)) as usize }

	/*
	* Remove the next 'n' bits from the bitbuffer variable.
	*/
	pub fn REMOVE_BITS(&mut self, n: usize) -> usize { self.bitbuf[self.idx as usize] >>= n; self.bitsleft[self.idx as usize] -= n as u32; self.bitsleft[self.idx as usize] as usize }

	/*
	* Remove and return the next 'n' bits from the bitbuffer variable.
	*/
	pub fn POP_BITS(&mut self, n: usize) -> usize { let tmp32 = self.BITS(n); self.REMOVE_BITS(n); tmp32 }

	/*
	* Reset GDeflate stream index.
	*/
	fn RESET(&mut self) { self.idx = 0 }

	/*
	* Load more bits from the input buffer until the specified number of bits is
	* present in the bitbuffer variable.  'n' cannot be too large; see MAX_ENSURE
	* and CAN_ENSURE().
	*/
	fn ENSURE_BITS(&self, n: u32, in_next: &mut *const u8) {
		if !self.HAVE_BITS(n) {
			self.bitbuf[self.idx as usize] |= (unsafe { *(*in_next as *const u32 ) } as u64) << self.bitsleft[self.idx as usize];
			*in_next = unsafe { (*in_next).add(BITS_PER_PACKET/8) };
			self.bitsleft[self.idx as usize] += BITS_PER_PACKET as u32;
		}
	}

	/*
	* Setup copy advance method depending on a number of streams used.
	*/
	fn ADVANCE_COPIES(&self, is_copy: u32) -> u32 {
		assert!(NUM_STREAMS == 32);
		is_copy.rotate_right(1)
	}

	/*
	* Advance GDeflate stream index. Refill bits if necessary.
	*/
	fn ADVANCE(&mut self, is_copy: u32, in_next: &mut *const u8)  {
		self.ENSURE_BITS(LOW_WATERMARK_BITS, in_next);
		self.idx = (self.idx + 1) % NUM_STREAMS as usize;
		self.ADVANCE_COPIES(is_copy);
	}
}

const LOW_WATERMARK_BITS: u32 = 32;
const HUFFDEC_SUBTABLE_POINTER: usize = 0x80000000;
const OFFSET_TABLEBITS: usize = 8;
const HUFFDEC_RESULT_SHIFT: u32 = 8;
const HUFFDEC_LENGTH_MASK: usize = 0xFF;
const HUFFDEC_EXTRA_OFFSET_BITS_SHIFT: usize = 16;
const HUFFDEC_OFFSET_BASE_MASK: usize = (1 << HUFFDEC_EXTRA_OFFSET_BITS_SHIFT) - 1;
const OFFSET_ENOUGH: usize = 402;

fn repeat_byte(b: u8) -> u64 {
	let mut v: u64;

	v = b as u64;
	v |= v << 8;
	v |= v << 16;
	v |= v << 32; // This only works on 64 bits.
	return v;
}

#[inline(always)]
fn do_copy(decompressor: &mut Decompressor, s: &mut gdeflate_state, out: &mut [u8], out_end: *mut u8) -> libdeflater::DecompressionError {
	let mut entry: usize;
	let mut offset: usize;
	let mut src: *const u8;
	let mut dst: *mut u8;
	let mut tmp32: u32;
	let length = s.copies[s.idx as usize].length;
	let out_next = s.copies[s.idx as usize].out_next as *mut u8;

	entry = decompressor.offset_decode_table[s.BITS(OFFSET_TABLEBITS) as usize] as usize;

	if entry & HUFFDEC_SUBTABLE_POINTER != 0 {
		s.REMOVE_BITS(OFFSET_TABLEBITS);
		entry = decompressor.offset_decode_table[(((entry >> HUFFDEC_RESULT_SHIFT) & 0xFFFF) + s.BITS(entry & HUFFDEC_LENGTH_MASK)) as usize] as usize;
	}

	s.REMOVE_BITS(entry & HUFFDEC_LENGTH_MASK);

	entry >>= HUFFDEC_RESULT_SHIFT;
	offset = (entry & HUFFDEC_OFFSET_BASE_MASK) + s.POP_BITS(entry >> HUFFDEC_EXTRA_OFFSET_BITS_SHIFT);

	unsafe {
		src = out_next.offset(-(offset as isize));
	}

	dst = out_next;

	let unaligned_access_is_fast = true; // Assuming unaligned access is fast in Rust
	let wordbytes = std::mem::size_of::<usize>();
	
	if unaligned_access_is_fast && unsafe { out_end.offset_from(out_next) } >= wordbytes as isize && length as usize >= wordbytes {
		if offset >= wordbytes {
			unsafe {
				while dst < out_next.offset(-(wordbytes as isize)) {
					//copy_word_unaligned(src, dst);
					*(dst as *mut u64) = *(src as *const u64);
					src = src.offset(wordbytes as isize);
					dst = dst.offset(wordbytes as isize);
				}
				while dst < out_next {
					*dst = *src;
					src = src.offset(1);
					dst = dst.offset(1);
				}
			}
		} else if offset == 1 {
			unsafe {
				let v = repeat_byte(*src);
				while dst < out_next.offset(-(wordbytes as isize)) {
					//store_word_unaligned(v, dst);
					*(dst as *mut u64) = v;
					dst = dst.offset(wordbytes as isize);
				}
				while dst < out_next {
					*dst = v as u8;
					dst = dst.offset(1);
				}
			}
		} else {
			unsafe {
				*dst = *src;
				dst = dst.offset(1);
				src = src.offset(1);
				*dst = *src;
				dst = dst.offset(1);
				src = src.offset(1);
				while dst < out_next {
					*dst = *src;
					dst = dst.offset(1);
					src = src.offset(1);
				}
			}
		}
	} else {
		unsafe {
			*dst = *src;
			dst = dst.offset(1);
			src = src.offset(1);
			*dst = *src;
			dst = dst.offset(1);
			src = src.offset(1);
			while dst < out_next {
				*dst = *src;
				dst = dst.offset(1);
				src = src.offset(1);
			}
		}
	}

	libdeflater::DecompressionError::BadData
}

const DEFLATE_NUM_PRECODE_SYMS: usize = 19;
const DEFLATE_NUM_LITLEN_SYMS: usize = 288;
const DEFLATE_NUM_OFFSET_SYMS: usize = 32;
const DEFLATE_MAX_PRE_CODEWORD_LEN: usize = 7;
const PRECODE_TABLEBITS: usize = 7;
const DEFLATE_MAX_LENS_OVERRUN: usize = 137;
const DEFLATE_BLOCKTYPE_UNCOMPRESSED: usize = 0;
const DEFLATE_BLOCKTYPE_STATIC_HUFFMAN: usize = 1;
const DEFLATE_BLOCKTYPE_DYNAMIC_HUFFMAN: usize = 2;
const HUFFDEC_LENGTH_BASE_SHIFT: u32 = 8;
const HUFFDEC_LITERAL: usize = 0x40000000;
const HUFFDEC_EXTRA_LENGTH_BITS_MASK: usize = 0xFF;
const LITLEN_TABLEBITS: usize = 10;
const HUFFDEC_END_OF_BLOCK_LENGTH: usize = 0;

/* Shift a decode result into its position in the decode table entry.  */
fn HUFFDEC_RESULT_ENTRY(result: usize) -> u32 { (result as u32) << HUFFDEC_RESULT_SHIFT }

/* The decode result for each precode symbol.  There is no special optimization
 * for the precode; the decode result is simply the symbol value.  */
static precode_decode_results: [u32; DEFLATE_NUM_PRECODE_SYMS] = [
	HUFFDEC_RESULT_ENTRY(0)   , HUFFDEC_RESULT_ENTRY(1)   , HUFFDEC_RESULT_ENTRY(2)   , HUFFDEC_RESULT_ENTRY(3)   ,
	HUFFDEC_RESULT_ENTRY(4)   , HUFFDEC_RESULT_ENTRY(5)   , HUFFDEC_RESULT_ENTRY(6)   , HUFFDEC_RESULT_ENTRY(7)   ,
	HUFFDEC_RESULT_ENTRY(8)   , HUFFDEC_RESULT_ENTRY(9)   , HUFFDEC_RESULT_ENTRY(10)  , HUFFDEC_RESULT_ENTRY(11)  ,
	HUFFDEC_RESULT_ENTRY(12)  , HUFFDEC_RESULT_ENTRY(13)  , HUFFDEC_RESULT_ENTRY(14)  , HUFFDEC_RESULT_ENTRY(15)  ,
	HUFFDEC_RESULT_ENTRY(16)  , HUFFDEC_RESULT_ENTRY(17)  , HUFFDEC_RESULT_ENTRY(18)  ,
];

/*
 * Build a table for fast decoding of symbols from a Huffman code.  As input,
 * this function takes the codeword length of each symbol which may be used in
 * the code.  As output, it produces a decode table for the canonical Huffman
 * code described by the codeword lengths.  The decode table is built with the
 * assumption that it will be indexed with "bit-reversed" codewords, where the
 * low-order bit is the first bit of the codeword.  This format is used for all
 * Huffman codes in DEFLATE.
 *
 * @decode_table
 *	The array in which the decode table will be generated.  This array must
 *	have sufficient length; see the definition of the ENOUGH numbers.
 * @lens
 *	An array which provides, for each symbol, the length of the
 *	corresponding codeword in bits, or 0 if the symbol is unused.  This may
 *	alias @decode_table, since nothing is written to @decode_table until all
 *	@lens have been consumed.  All codeword lengths are assumed to be <=
 *	@max_codeword_len but are otherwise considered untrusted.  If they do
 *	not form a valid Huffman code, then the decode table is not built and
 *	%false is returned.
 * @num_syms
 *	The number of symbols in the code, including all unused symbols.
 * @decode_results
 *	An array which provides, for each symbol, the actual value to store into
 *	the decode table.  This value will be directly produced as the result of
 *	decoding that symbol, thereby moving the indirection out of the decode
 *	loop and into the table initialization.
 * @table_bits
 *	The log base-2 of the number of main table entries to use.
 * @max_codeword_len
 *	The maximum allowed codeword length for this Huffman code.
 *	Must be <= DEFLATE_MAX_CODEWORD_LEN.
 * @sorted_syms
 *	A temporary array of length @num_syms.
 *
 * Returns %true if successful; %false if the codeword lengths do not form a
 * valid Huffman code.
 */
fn build_decode_table(u32 decode_table[], const len_t lens[], const unsigned num_syms, const u32 decode_results[], const unsigned table_bits, const unsigned max_codeword_len, u16 *sorted_syms) -> bool
{
	unsigned len_counts[DEFLATE_MAX_CODEWORD_LEN + 1];
	unsigned offsets[DEFLATE_MAX_CODEWORD_LEN + 1];
	unsigned sym;		/* current symbol */
	unsigned codeword;	/* current codeword, bit-reversed */
	unsigned len;		/* current codeword length in bits */
	unsigned count;		/* num codewords remaining with this length */
	u32 codespace_used;	/* codespace used out of '2^max_codeword_len' */
	unsigned cur_table_end; /* end index of current table */
	unsigned subtable_prefix; /* codeword prefix of current subtable */
	unsigned subtable_start;  /* start index of current subtable */
	unsigned subtable_bits;   /* log2 of current subtable length */

	/* Count how many codewords have each length, including 0. */
	for (len = 0; len <= max_codeword_len; len++)
		len_counts[len] = 0;
	for (sym = 0; sym < num_syms; sym++)
		len_counts[lens[sym]]++;

	/*
	 * Sort the symbols primarily by increasing codeword length and
	 * secondarily by increasing symbol value; or equivalently by their
	 * codewords in lexicographic order, since a canonical code is assumed.
	 *
	 * For efficiency, also compute 'codespace_used' in the same pass over
	 * 'len_counts[]' used to build 'offsets[]' for sorting.
	 */

	/* Ensure that 'codespace_used' cannot overflow. */
	STATIC_ASSERT(sizeof(codespace_used) == 4);
	STATIC_ASSERT(UINT32_MAX / (1U << (DEFLATE_MAX_CODEWORD_LEN - 1)) >=
		      DEFLATE_MAX_NUM_SYMS);

	offsets[0] = 0;
	offsets[1] = len_counts[0];
	codespace_used = 0;
	for (len = 1; len < max_codeword_len; len++) {
		offsets[len + 1] = offsets[len] + len_counts[len];
		codespace_used = (codespace_used << 1) + len_counts[len];
	}
	codespace_used = (codespace_used << 1) + len_counts[len];

	for (sym = 0; sym < num_syms; sym++)
		sorted_syms[offsets[lens[sym]]++] = sym;

	sorted_syms += offsets[0]; /* Skip unused symbols */

	/* lens[] is done being used, so we can write to decode_table[] now. */

	/*
	 * Check whether the lengths form a complete code (exactly fills the
	 * codespace), an incomplete code (doesn't fill the codespace), or an
	 * overfull code (overflows the codespace).  A codeword of length 'n'
	 * uses proportion '1/(2^n)' of the codespace.  An overfull code is
	 * nonsensical, so is considered invalid.  An incomplete code is
	 * considered valid only in two specific cases; see below.
	 */

	/* overfull code? */
	if (unlikely(codespace_used > (1U << max_codeword_len)))
		return false;

	/* incomplete code? */
	if (unlikely(codespace_used < (1U << max_codeword_len))) {
		u32 entry;
		unsigned i;

		if (codespace_used == 0) {
			/*
			 * An empty code is allowed.  This can happen for the
			 * offset code in DEFLATE, since a dynamic Huffman block
			 * need not contain any matches.
			 */

			/* sym=0, len=1 (arbitrary) */
			entry = decode_results[0] | 1;
		} else {
			/*
			 * Allow codes with a single used symbol, with codeword
			 * length 1.  The DEFLATE RFC is unclear regarding this
			 * case.  What zlib's decompressor does is permit this
			 * for the litlen and offset codes and assume the
			 * codeword is '0' rather than '1'.  We do the same
			 * except we allow this for precodes too, since there's
			 * no convincing reason to treat the codes differently.
			 * We also assign both codewords '0' and '1' to the
			 * symbol to avoid having to handle '1' specially.
			 */
			if (codespace_used != (1U << (max_codeword_len - 1)) ||
			    len_counts[1] != 1)
				return false;
			entry = decode_results[*sorted_syms] | 1;
		}
		/*
		 * Note: the decode table still must be fully initialized, in
		 * case the stream is malformed and contains bits from the part
		 * of the codespace the incomplete code doesn't use.
		 */
		for (i = 0; i < (1U << table_bits); i++)
			decode_table[i] = entry;
		return true;
	}

	/*
	 * The lengths form a complete code.  Now, enumerate the codewords in
	 * lexicographic order and fill the decode table entries for each one.
	 *
	 * First, process all codewords with len <= table_bits.  Each one gets
	 * '2^(table_bits-len)' direct entries in the table.
	 *
	 * Since DEFLATE uses bit-reversed codewords, these entries aren't
	 * consecutive but rather are spaced '2^len' entries apart.  This makes
	 * filling them naively somewhat awkward and inefficient, since strided
	 * stores are less cache-friendly and preclude the use of word or
	 * vector-at-a-time stores to fill multiple entries per instruction.
	 *
	 * To optimize this, we incrementally double the table size.  When
	 * processing codewords with length 'len', the table is treated as
	 * having only '2^len' entries, so each codeword uses just one entry.
	 * Then, each time 'len' is incremented, the table size is doubled and
	 * the first half is copied to the second half.  This significantly
	 * improves performance over naively doing strided stores.
	 *
	 * Note that some entries copied for each table doubling may not have
	 * been initialized yet, but it doesn't matter since they're guaranteed
	 * to be initialized later (because the Huffman code is complete).
	 */
	codeword = 0;
	len = 1;
	while ((count = len_counts[len]) == 0)
		len++;
	cur_table_end = 1U << len;
	while (len <= table_bits) {
		/* Process all 'count' codewords with length 'len' bits. */
		do {
			unsigned bit;

			/* Fill the first entry for the current codeword. */
			decode_table[codeword] =
				decode_results[*sorted_syms++] | len;

			if (codeword == cur_table_end - 1) {
				/* Last codeword (all 1's) */
				for (; len < table_bits; len++) {
					memcpy(&decode_table[cur_table_end],
					       decode_table,
					       cur_table_end *
						sizeof(decode_table[0]));
					cur_table_end <<= 1;
				}
				return true;
			}
			/*
			 * To advance to the lexicographically next codeword in
			 * the canonical code, the codeword must be incremented,
			 * then 0's must be appended to the codeword as needed
			 * to match the next codeword's length.
			 *
			 * Since the codeword is bit-reversed, appending 0's is
			 * a no-op.  However, incrementing it is nontrivial.  To
			 * do so efficiently, use the 'bsr' instruction to find
			 * the last (highest order) 0 bit in the codeword, set
			 * it, and clear any later (higher order) 1 bits.  But
			 * 'bsr' actually finds the highest order 1 bit, so to
			 * use it first flip all bits in the codeword by XOR'ing
			 * it with (1U << len) - 1 == cur_table_end - 1.
			 */
			bit = 1U << bsr32(codeword ^ (cur_table_end - 1));
			codeword &= bit - 1;
			codeword |= bit;
		} while (--count);

		/* Advance to the next codeword length. */
		do {
			if (++len <= table_bits) {
				memcpy(&decode_table[cur_table_end],
				       decode_table,
				       cur_table_end * sizeof(decode_table[0]));
				cur_table_end <<= 1;
			}
		} while ((count = len_counts[len]) == 0);
	}

	/* Process codewords with len > table_bits.  These require subtables. */
	cur_table_end = 1U << table_bits;
	subtable_prefix = -1;
	subtable_start = 0;
	for (;;) {
		u32 entry;
		unsigned i;
		unsigned stride;
		unsigned bit;

		/*
		 * Start a new subtable if the first 'table_bits' bits of the
		 * codeword don't match the prefix of the current subtable.
		 */
		if ((codeword & ((1U << table_bits) - 1)) != subtable_prefix) {
			subtable_prefix = (codeword & ((1U << table_bits) - 1));
			subtable_start = cur_table_end;
			/*
			 * Calculate the subtable length.  If the codeword has
			 * length 'table_bits + n', then the subtable needs
			 * '2^n' entries.  But it may need more; if fewer than
			 * '2^n' codewords of length 'table_bits + n' remain,
			 * then the length will need to be incremented to bring
			 * in longer codewords until the subtable can be
			 * completely filled.  Note that because the Huffman
			 * code is complete, it will always be possible to fill
			 * the subtable eventually.
			 */
			subtable_bits = len - table_bits;
			codespace_used = count;
			while (codespace_used < (1U << subtable_bits)) {
				subtable_bits++;
				codespace_used = (codespace_used << 1) +
					len_counts[table_bits + subtable_bits];
			}
			cur_table_end = subtable_start + (1U << subtable_bits);

			/*
			 * Create the entry that points from the main table to
			 * the subtable.  This entry contains the index of the
			 * start of the subtable and the number of bits with
			 * which the subtable is indexed (the log base 2 of the
			 * number of entries it contains).
			 */
			decode_table[subtable_prefix] =
				HUFFDEC_SUBTABLE_POINTER |
				HUFFDEC_RESULT_ENTRY(subtable_start) |
				subtable_bits;
		}

		/* Fill the subtable entries for the current codeword. */
		entry = decode_results[*sorted_syms++] | (len - table_bits);
		i = subtable_start + (codeword >> table_bits);
		stride = 1U << (len - table_bits);
		do {
			decode_table[i] = entry;
			i += stride;
		} while (i < cur_table_end);

		/* Advance to the next codeword. */
		if (codeword == (1U << len) - 1) /* last codeword (all 1's)? */
			return true;
		bit = 1U << bsr32(codeword ^ ((1U << len) - 1));
		codeword &= bit - 1;
		codeword |= bit;
		count--;
		while (count == 0)
			count = len_counts[++len];
	}
}

/* Build the decode table for the precode.  */
fn build_precode_decode_table(d: &mut Decompressor) -> bool {
	/* When you change TABLEBITS, you must change ENOUGH, and vice versa! */
	assert!(PRECODE_TABLEBITS == 7 && PRECODE_ENOUGH == 128);

	return build_decode_table(d.u.l.precode_decode_table, d.u.precode_lens, DEFLATE_NUM_PRECODE_SYMS, precode_decode_results, PRECODE_TABLEBITS, DEFLATE_MAX_PRE_CODEWORD_LEN, d.sorted_syms);
}

fn FUNCNAME(decompressor: &mut Decompressor, in_: *const u8, in_nbytes: usize, out: *mut u8, out_nbytes_avail: usize, actual_in_nbytes_ret: &mut usize, actual_out_nbytes_ret: &mut usize) {
	let out_next = out;
	let out_end = unsafe { out_next.add(out_nbytes_avail) };
	let in_next = in_;
	let in_end = unsafe { in_next.add(in_nbytes) };
	let mut state = gdeflate_state::new();
	
	let i;
	let is_final_block;
	let block_type;
	let len;
	let num_litlen_syms;
	let num_offset_syms;
	let tmp32;
	let is_copy = 0;

	/* Starting to read GDeflate stream.  */
	state.RESET();

	for n in 0..NUM_STREAMS {
		state.bitbuf[n] = 0;
		state.bitsleft[n] = 0;
		state.copies[n].length = 0;
		state.ADVANCE(is_copy, &mut in_next); // Will advance pointer
	}

	let IS_COPY = || is_copy & 1;
	let COPY_COMPLETE = || is_copy &= !1;

	let next_block = || {
		/* Starting to read the next block.  */
		state.RESET();

		/* BFINAL: 1 bit  */
		is_final_block = state.POP_BITS(1);

		/* BTYPE: 2 bits  */
		block_type = state.POP_BITS(2);

		state.ENSURE_BITS(LOW_WATERMARK_BITS, &mut in_next);

		if block_type == DEFLATE_BLOCKTYPE_DYNAMIC_HUFFMAN {
			/* Dynamic Huffman block.  */

			/* The order in which precode lengths are stored.  */
			static deflate_precode_lens_permutation: [u8; DEFLATE_NUM_PRECODE_SYMS as usize] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

			let mut num_explicit_precode_lens;

			/* Read the codeword length counts.  */

			assert!(DEFLATE_NUM_LITLEN_SYMS == ((1 << 5) - 1) + 257);
			num_litlen_syms = state.POP_BITS(5) + 257;

			assert!(DEFLATE_NUM_OFFSET_SYMS == ((1 << 5) - 1) + 1);
			num_offset_syms = state.POP_BITS(5) + 1;

			assert!(DEFLATE_NUM_PRECODE_SYMS == ((1 << 4) - 1) + 4);
			num_explicit_precode_lens = state.POP_BITS(4) + 4;

			decompressor.static_codes_loaded = false;

			state.ENSURE_BITS(LOW_WATERMARK_BITS, &mut in_next);

			/* Read the precode codeword lengths.  */
			assert!(DEFLATE_MAX_PRE_CODEWORD_LEN == (1 << 3) - 1);
			for i in 0..num_explicit_precode_lens {
				decompressor.u.precode_lens[deflate_precode_lens_permutation[i] as usize] = state.POP_BITS(3) as u8; // WHAT?

				state.ADVANCE(is_copy, &mut in_next);
			}

			for i in num_explicit_precode_lens..DEFLATE_NUM_PRECODE_SYMS {
				decompressor.u.precode_lens[deflate_precode_lens_permutation[i] as usize] = 0;
			}

			/* Build the decode table for the precode.  */
			assert!(build_precode_decode_table(&mut decompressor));

			state.RESET();

			/* Expand the literal/length and offset codeword lengths.  */
			for i in 0..((num_litlen_syms + num_offset_syms) as usize) {
				let mut entry: usize;
				let mut presym;
				let mut rep_val;
				let mut rep_count;

				/* (The code below assumes that the precode decode table
				* does not have any subtables.)  */
				assert!(PRECODE_TABLEBITS == DEFLATE_MAX_PRE_CODEWORD_LEN);

				/* Read the next precode symbol.  */
				entry = decompressor.u.l.precode_decode_table[state.BITS(DEFLATE_MAX_PRE_CODEWORD_LEN)] as usize;
				state.REMOVE_BITS(entry & HUFFDEC_LENGTH_MASK);

				presym = entry >> HUFFDEC_RESULT_SHIFT;

				if presym < 16 {
					/* Explicit codeword length  */
					decompressor.u.l.lens[i] = presym as u8; // Weird cast
					i += 1;
					state.ADVANCE(is_copy, &mut in_next);
					continue;
				}

				/* Run-length encoded codeword lengths  */

				/* Note: we don't need verify that the repeat count
				* doesn't overflow the number of elements, since we
				* have enough extra spaces to allow for the worst-case
				* overflow (138 zeroes when only 1 length was
				* remaining).
				*
				* In the case of the small repeat counts (presyms 16
				* and 17), it is fastest to always write the maximum
				* number of entries.  That gets rid of branches that
				* would otherwise be required.
				*
				* It is not just because of the numerical order that
				* our checks go in the order 'presym < 16', 'presym ==
				* 16', and 'presym == 17'.  For typical data this is
				* ordered from most frequent to least frequent case.
				*/
				assert!(DEFLATE_MAX_LENS_OVERRUN == 138 - 1);

				if presym == 16 {
					/* Repeat the previous length 3 - 6 times  */
					assert!(i != 0);
					rep_val = decompressor.u.l.lens[i - 1];
					assert!(3 + ((1 << 2) - 1) == 6);
					rep_count = 3 + state.POP_BITS(2);
					decompressor.u.l.lens[i + 0] = rep_val;
					decompressor.u.l.lens[i + 1] = rep_val;
					decompressor.u.l.lens[i + 2] = rep_val;
					decompressor.u.l.lens[i + 3] = rep_val;
					decompressor.u.l.lens[i + 4] = rep_val;
					decompressor.u.l.lens[i + 5] = rep_val;
					i += rep_count;
				} else if presym == 17 {
					/* Repeat zero 3 - 10 times  */
					assert!(3 + ((1 << 3) - 1) == 10);
					rep_count = 3 + state.POP_BITS(3);
					decompressor.u.l.lens[i + 0] = 0;
					decompressor.u.l.lens[i + 1] = 0;
					decompressor.u.l.lens[i + 2] = 0;
					decompressor.u.l.lens[i + 3] = 0;
					decompressor.u.l.lens[i + 4] = 0;
					decompressor.u.l.lens[i + 5] = 0;
					decompressor.u.l.lens[i + 6] = 0;
					decompressor.u.l.lens[i + 7] = 0;
					decompressor.u.l.lens[i + 8] = 0;
					decompressor.u.l.lens[i + 9] = 0;
					i += rep_count;
				} else {
					/* Repeat zero 11 - 138 times  */
					assert!(11 + ((1 << 7) - 1) == 138);
					rep_count = 11 + state.POP_BITS(7);
					memset(&decompressor.u.l.lens[i], 0, rep_count * sizeof(decompressor.u.l.lens[i]));
					i += rep_count;
				}

				state.ADVANCE(is_copy, &mut in_next);
			}
		} else if block_type == DEFLATE_BLOCKTYPE_UNCOMPRESSED {

			/* Uncompressed block: copy 'len' bytes literally from the input
			* buffer to the output buffer.  */

			/* Count bits in the bit buffers. */
			let mut num_buffered_bits = 0;
			for n in 0..NUM_STREAMS {
				num_buffered_bits += state.bitsleft[n];
			}

			unsafe {
				assert!(in_end.sub_ptr(in_next).add((num_buffered_bits as usize + 7)/8) as usize >= 2);
			}

			len = state.POP_BITS(16);

			if unsafe { len > out_end.sub_ptr(out_next) } {
				return; // LIBDEFLATE_INSUFFICIENT_SPACE;
			}

			unsafe {
				assert!(len <= in_end.sub_ptr(in_next).add((num_buffered_bits as usize + 7)/8));
			}				

			while len != 0 {
				unsafe {
					*out_next = state.POP_BITS(8) as u8; // Weird cast
					out_next = out_next.add(1);
				}

				len -= 1;
				state.ADVANCE(is_copy, &mut in_next);
			}

			block_done();
		} else {
			assert!(block_type == DEFLATE_BLOCKTYPE_STATIC_HUFFMAN);

			/*
			* Static Huffman block: build the decode tables for the static
			* codes.  Skip doing so if the tables are already set up from
			* an earlier static block; this speeds up decompression of
			* degenerate input of many empty or very short static blocks.
			*
			* Afterwards, the remainder is the same as decompressing a
			* dynamic Huffman block.
			*/

			if decompressor.static_codes_loaded {
				have_decode_tables();
			}

			decompressor.static_codes_loaded = true;

			assert!(DEFLATE_NUM_LITLEN_SYMS == 288);
			assert!(DEFLATE_NUM_OFFSET_SYMS == 32);

			decompressor.u.l.lens[0..144] = [8u8; 144];
			decompressor.u.l.lens[144..256] = 9u8;
			decompressor.u.l.lens[256..280] = 7u8;
			decompressor.u.l.lens[280..288] = 8u8;

			decompressor.u.l.lens[288..288 + 32] = 5u8;

			num_litlen_syms = 288;
			num_offset_syms = 32;
		}

		/* Decompressing a Huffman block (either dynamic or static)  */

		assert!(build_offset_decode_table(d, num_litlen_syms, num_offset_syms));
		assert!(build_litlen_decode_table(d, num_litlen_syms, num_offset_syms));
	};

	let block_done = || {
		/* Run the outstanding deferred copies.  */

		for n in 0..NUM_STREAMS {
			if IS_COPY() != 0 {
				do_copy(decompressor, &mut state, out, out_end);

				COPY_COMPLETE();
			}

			state.ADVANCE(is_copy as u32, &mut in_next);
		}

		/* Finished decoding a block.  */

		if !(is_final_block != 0) {
			next_block();
		}
	};

	let have_decode_tables = || {
		state.RESET();

		/*
		* Stores a deferred copy in current GDeflate stream.
		*/
		let STORE_COPY = |len: usize, out: *mut u8| {
			state.copies[state.idx].length = len as u32;
			state.copies[state.idx].out_next = out as *mut u32;
			is_copy |= 1;
		};

		/* The main GDEFLATE decode loop  */
		loop {
			let mut entry: usize;
			let mut length;

			if IS_COPY() == 0 { // WHATCH OUT
				/* Decode a litlen symbol.  */
				entry = decompressor.u.litlen_decode_table[state.BITS(LITLEN_TABLEBITS)] as usize;

				if (entry & HUFFDEC_SUBTABLE_POINTER) != 0 {
					/* Litlen subtable required (uncommon case)  */
					state.REMOVE_BITS(LITLEN_TABLEBITS);
					entry = decompressor.u.litlen_decode_table[((entry >> HUFFDEC_RESULT_SHIFT) & 0xFFFF) + state.BITS(entry & HUFFDEC_LENGTH_MASK)] as usize;
				}

				state.REMOVE_BITS(entry & HUFFDEC_LENGTH_MASK);

				if (entry & HUFFDEC_LITERAL) != 0 {
					/* Literal  */
					if out_next == out_end {
						return; // LIBDEFLATE_INSUFFICIENT_SPACE;
					}

					unsafe { 
						*out_next = (entry >> HUFFDEC_RESULT_SHIFT) as u8;
						out_next = out_end.add(1);
					}

					state.ADVANCE(is_copy, &mut in_next);

					continue;
				}

				/* Match or end-of-block  */

				entry >>= HUFFDEC_RESULT_SHIFT;

				/* Pop the extra length bits and add them to the length base to
				* produce the full length.  */
				length = (entry >> HUFFDEC_LENGTH_BASE_SHIFT) + state.POP_BITS(entry & HUFFDEC_EXTRA_LENGTH_BITS_MASK);

				/* The match destination must not end after the end of the
				* output buffer.  For efficiency, combine this check with the
				* end-of-block check.  We're using 0 for the special
				* end-of-block length, so subtract 1 and it turn it into
				* SIZE_MAX.  */
				assert!(HUFFDEC_END_OF_BLOCK_LENGTH == 0);
				if length - 1 >= unsafe { out_end.sub_ptr(out_next) } {
					if length != HUFFDEC_END_OF_BLOCK_LENGTH {
						return; // LIBDEFLATE_INSUFFICIENT_SPACE;
					}

					block_done();
				}

				/* Store copy for use later.  */
				STORE_COPY(length, out_next);

				/* Advance output stream.  */
				unsafe { out_next = out_next.add(length); }
			} else {
				let res = do_copy(decompressor, &mut state, out, out_end);

				COPY_COMPLETE();
			}

			state.ADVANCE(is_copy, &mut in_next);
		}
	};

	/* That was the last block.  */

	/* Optionally return the actual number of bytes read */
	*actual_in_nbytes_ret = unsafe { in_next.sub_ptr(in_); }

	/* Optionally return the actual number of bytes written */
	*actual_out_nbytes_ret = unsafe { out_next.sub_ptr(out); }

	if out_next != out_end {
		return; // LIBDEFLATE_SHORT_OUTPUT;
	}

	return; // LIBDEFLATE_SUCCESS;
}

struct libdeflate_gdeflate_in_page {
	/* Compressed GDEFLATE page data. */
	data: *const u8,
	/* Size in bytes of compressed GDEFLATE page. */
	nbytes: usize,
}

fn libdeflate_gdeflate_decompress(decompressor: &mut Decompressor, in_pages: &[libdeflate_gdeflate_in_page], out: *mut u8, out_nbytes_avail: usize, actual_out_nbytes_ret: &mut usize)
{
	let out_bytes = out;

	if in_pages.len() == 0 {
		return; // LIBDEFLATE_BAD_DATA;
	}

	for npage in  0..in_pages.len() {
		let page_out_nbytes_ret: usize; let page_in_nbytes_ret: usize;

		decompress_impl(decompressor, in_pages[npage].data, in_pages[npage].nbytes, out_bytes, out_nbytes_avail, &page_in_nbytes_ret, &page_out_nbytes_ret);

		out_bytes += page_out_nbytes_ret;
		out_nbytes_avail -= page_out_nbytes_ret;

		*actual_out_nbytes_ret += page_out_nbytes_ret;
	}

	return;
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_compressor() {
		let mut c = libdeflate_alloc_gdeflate_compressor(12);

		let mut vec = Vec::with_capacity(65536*2);

		for i in 0..65536*2 {
			vec.push(i as u8);
		}

		let mut page1 = Vec::with_capacity(66000);
		let mut page2 = Vec::with_capacity(66000);

		unsafe {
			page1.set_len(66000);
			page2.set_len(66000);
		}

		let final_size = gdeflate_compress(&mut c, &vec, &[page1.as_mut_slice(), page2.as_mut_slice()]);

		assert!(final_size > 0);

		let decompressor = Decompressor::new();

		let pages = [
			libdeflate_gdeflate_in_page {
				data: page1.as_ptr(),
				nbytes: 66000
			},
			libdeflate_gdeflate_in_page {
				data: page2.as_ptr(),
				nbytes: 66000
			}
		];

		let out = vec![0u8; 65536*2];

		unsafe {
			out.set_len(65536*2);
		}

		let mut actual_out_nbytes_ret: usize = 0;

		libdeflate_gdeflate_decompress(decompressor, &pages, out.as_mut_ptr(), 65536*2, &mut actual_out_nbytes_ret);

		assert_eq!(actual_out_nbytes_ret, 65536*2);

		for (a, b) in vec.iter().zip(out.iter()) {
			assert_eq!(a, b);
		}
	}
}