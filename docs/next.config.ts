import type { NextConfig } from 'next';
import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();
	
const config: NextConfig = {
	reactStrictMode: true,
	async rewrites() {
		return [
			{
				source: '/docs/:path*.mdx',
				destination: '/llms.mdx/:path*',
			},
		];
	},
	output: 'standalone',
};

export default withMDX(config);

import { initOpenNextCloudflareForDev } from "@opennextjs/cloudflare";
initOpenNextCloudflareForDev();