import { createRouter as createTanStackRouter } from '@tanstack/react-router';
import { routeTree } from './routeTree.gen';
import { NotFound } from '@/components/not-found';

// The getRouter function creates and returns the application router instance
export function getRouter() {
	return createTanStackRouter({
		routeTree,
		defaultPreload: 'intent',
		scrollRestoration: true,
		trailingSlash: 'never',
		defaultNotFoundComponent: NotFound,
	});
}

declare module '@tanstack/react-router' {
	interface Register {
		router: ReturnType<typeof getRouter>;
	}
}
