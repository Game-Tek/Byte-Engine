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
};

export default withMDX(config);
