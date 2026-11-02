// Prints the Metal compiler's statistics for each compute function in the given metallibs: temporary registers,
// spilled bytes, and instruction counts, from Metal's private pipeline option 1 << 18. The M4 caps temporaries at 96
// a thread and spills past them.
//
// build: clang -fobjc-arc -framework Metal -framework Foundation pipeline_statistics.m -o pipeline_statistics
// usage: pipeline_statistics <file.metallib>...
//   The viewer's baked `*-raw` resources that start with `MTLB` are metallibs and load as they are; find a pass's with
//   `grep -l -a <binding name> <resources>/*-raw` and take the newest, since a rebake leaves the older ones behind.
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>

int main(int argc, const char **argv) {
	@autoreleasepool {
		id<MTLDevice> device = MTLCreateSystemDefaultDevice();
		for (int i = 1; i < argc; i++) {
			NSError *error = nil;
			id<MTLLibrary> library = [device newLibraryWithURL:[NSURL fileURLWithPath:@(argv[i])] error:&error];
			if (!library) { NSLog(@"%s: %@", argv[i], error); continue; }
			for (NSString *name in library.functionNames) {
				id<MTLFunction> function = [library newFunctionWithName:name];
				if (function.functionType != MTLFunctionTypeKernel) continue;
				MTLComputePipelineReflection *reflection = nil;
				id<MTLComputePipelineState> state = [device newComputePipelineStateWithFunction:function
					options:(MTLPipelineOption)(1ull << 18) reflection:&reflection error:&error];
				if (!state) { NSLog(@"%@: %@", name, error); continue; }
				NSDictionary *statistics = [(id)reflection performSelector:@selector(performanceStatistics)];
				printf("== %s %s (max threads %lu)\n", argv[i], name.UTF8String,
					(unsigned long)state.maxTotalThreadsPerThreadgroup);
				for (NSString *key in [statistics.allKeys sortedArrayUsingSelector:@selector(compare:)]) {
					if ([key isEqualToString:@"Remarks"]) continue;
					printf("  %-40s %s\n", key.UTF8String, [[statistics[key] description] UTF8String]);
				}
			}
		}
	}
	return 0;
}
