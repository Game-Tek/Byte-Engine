// Prints the Metal compiler's statistics for each kernel and tile function in the given metallibs: temporary
// registers, spilled bytes, threadgroup memory, and instruction counts, from Metal's private pipeline option 1 << 18.
// The M4 caps temporaries at 96 a thread and spills past them.
//
// build: clang -fobjc-arc -framework Metal -framework Foundation pipeline_statistics.m -o pipeline_statistics
// usage: pipeline_statistics [--json] [--tile-formats rgba16f,r32f,...] <file.metallib>...
//   Get the metallibs from a dry-run bake, named after their shaders:
//     beld bake --dry-run --intercept '*.metallib' --intercept-out <dir> <asset IDs>
//   `--json` prints one array of {file, function, kind, max_threads, attachments, statistics} objects instead of text,
//   so before/after runs compare with a JSON diff.
//
// Tile functions read and write an imageblock, which only a tile render pipeline lowers; a compute pipeline rejects
// them with "unlowered function call to air.store.implicit_imageblock". A kernel that fails as compute is built as a
// tile function with the attachment formats `--tile-formats` lists. Without the flag, it tries one to eight rgba16f
// attachments and keeps the first count Metal accepts. The statistics barely depend on the formats, but pass the
// pass's real formats when a store conversion matters.
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>

static MTLPixelFormat parse_format(NSString *name) {
	NSDictionary<NSString *, NSNumber *> *formats = @{
		@"r8" : @(MTLPixelFormatR8Unorm), @"rg8" : @(MTLPixelFormatRG8Unorm), @"rgba8" : @(MTLPixelFormatRGBA8Unorm),
		@"r16f" : @(MTLPixelFormatR16Float), @"rg16f" : @(MTLPixelFormatRG16Float), @"rgba16f" : @(MTLPixelFormatRGBA16Float),
		@"r32f" : @(MTLPixelFormatR32Float), @"rg32f" : @(MTLPixelFormatRG32Float), @"rgba32f" : @(MTLPixelFormatRGBA32Float),
		@"r32u" : @(MTLPixelFormatR32Uint), @"rg11b10f" : @(MTLPixelFormatRG11B10Float),
	};
	NSNumber *format = formats[name.lowercaseString];
	if (!format) {
		fprintf(stderr, "Unknown tile format '%s'. Use one of: %s.\n", name.UTF8String,
			[[formats.allKeys sortedArrayUsingSelector:@selector(compare:)] componentsJoinedByString:@", "].UTF8String);
		exit(2);
	}
	return (MTLPixelFormat)format.unsignedIntegerValue;
}

// Builds `function` as a tile render pipeline with `formats` as its color attachments.
static id<MTLRenderPipelineState> tile_state(id<MTLDevice> device, id<MTLFunction> function, NSArray<NSNumber *> *formats,
	MTLRenderPipelineReflection **reflection, NSError **error) {
	MTLTileRenderPipelineDescriptor *descriptor = [MTLTileRenderPipelineDescriptor new];
	descriptor.tileFunction = function;
	// The engine's tile functions run fewer threads than the tile has pixels, as ghi builds them.
	descriptor.threadgroupSizeMatchesTileSize = NO;
	for (NSUInteger index = 0; index < formats.count; index++) {
		descriptor.colorAttachments[index].pixelFormat = (MTLPixelFormat)formats[index].unsignedIntegerValue;
	}
	return [device newRenderPipelineStateWithTileDescriptor:descriptor options:(MTLPipelineOption)(1ull << 18)
		reflection:reflection error:error];
}

int main(int argc, const char **argv) {
	@autoreleasepool {
		BOOL json = NO;
		NSArray<NSNumber *> *tile_formats = nil;
		NSMutableArray<NSString *> *paths = [NSMutableArray array];
		for (int i = 1; i < argc; i++) {
			NSString *argument = @(argv[i]);
			if ([argument isEqualToString:@"--json"]) {
				json = YES;
			} else if ([argument isEqualToString:@"--tile-formats"] && i + 1 < argc) {
				NSMutableArray<NSNumber *> *formats = [NSMutableArray array];
				for (NSString *name in [@(argv[++i]) componentsSeparatedByString:@","]) [formats addObject:@(parse_format(name))];
				tile_formats = formats;
			} else {
				[paths addObject:argument];
			}
		}

		id<MTLDevice> device = MTLCreateSystemDefaultDevice();
		NSMutableArray<NSDictionary *> *results = [NSMutableArray array];
		int failures = 0;
		for (NSString *path in paths) {
			NSError *error = nil;
			id<MTLLibrary> library = [device newLibraryWithURL:[NSURL fileURLWithPath:path] error:&error];
			if (!library) { NSLog(@"%@: %@", path, error); failures++; continue; }
			for (NSString *name in library.functionNames) {
				id<MTLFunction> function = [library newFunctionWithName:name];
				if (function.functionType != MTLFunctionTypeKernel) continue;

				NSString *kind = @"compute";
				NSUInteger max_threads = 0;
				NSArray<NSNumber *> *attachments = @[];
				id reflection = nil;
				MTLComputePipelineReflection *compute_reflection = nil;
				id<MTLComputePipelineState> compute = [device newComputePipelineStateWithFunction:function
					options:(MTLPipelineOption)(1ull << 18) reflection:&compute_reflection error:&error];
				if (compute) {
					reflection = compute_reflection;
					max_threads = compute.maxTotalThreadsPerThreadgroup;
				} else {
					// Try the given formats, or one to eight rgba16f attachments until Metal accepts a count.
					NSMutableArray<NSArray<NSNumber *> *> *candidates = [NSMutableArray array];
					if (tile_formats) {
						[candidates addObject:tile_formats];
					} else {
						for (int count = 1; count <= 8; count++) {
							NSMutableArray<NSNumber *> *formats = [NSMutableArray array];
							for (int index = 0; index < count; index++) [formats addObject:@(MTLPixelFormatRGBA16Float)];
							[candidates addObject:formats];
						}
					}
					for (NSArray<NSNumber *> *formats in candidates) {
						MTLRenderPipelineReflection *tile_reflection = nil;
						id<MTLRenderPipelineState> tile = tile_state(device, function, formats, &tile_reflection, &error);
						if (!tile) continue;
						kind = @"tile";
						reflection = tile_reflection;
						max_threads = tile.maxTotalThreadsPerThreadgroup;
						attachments = formats;
						break;
					}
				}
				if (!reflection || ![reflection respondsToSelector:@selector(performanceStatistics)]) {
					NSLog(@"%@ %@: %@", path, name, error);
					failures++;
					continue;
				}

				NSMutableDictionary *statistics = [[reflection performSelector:@selector(performanceStatistics)] mutableCopy];
				[statistics removeObjectForKey:@"Remarks"];
				[results addObject:@{
					@"file" : path,
					@"function" : name,
					@"kind" : kind,
					@"max_threads" : @(max_threads),
					@"attachments" : attachments,
					@"statistics" : statistics,
				}];
			}
		}

		if (json) {
			NSError *error = nil;
			NSData *data = [NSJSONSerialization dataWithJSONObject:results
				options:NSJSONWritingPrettyPrinted | NSJSONWritingSortedKeys error:&error];
			if (!data) { NSLog(@"%@", error); return 1; }
			fwrite(data.bytes, 1, data.length, stdout);
			printf("\n");
		} else {
			for (NSDictionary *result in results) {
				printf("== %s %s (%s, max threads %lu)\n", [result[@"file"] UTF8String], [result[@"function"] UTF8String],
					[result[@"kind"] UTF8String], [result[@"max_threads"] unsignedLongValue]);
				NSDictionary *statistics = result[@"statistics"];
				for (NSString *key in [statistics.allKeys sortedArrayUsingSelector:@selector(compare:)]) {
					printf("  %-40s %s\n", key.UTF8String, [[statistics[key] description] UTF8String]);
				}
			}
		}
		return failures == 0 ? 0 : 1;
	}
}
